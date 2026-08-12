pub mod render;
mod runtime;
mod terminal;

use crate::agent::follower::{FollowerEvent, TranscriptFollower};
use crate::agent::{
    AgentKind, AgentPaths, AgentStatus, TranscriptAdapter, agent_identity, resolve_transcript,
};
use crate::app::{AppEvent, AppState};
use crate::editor::{Editor, staged_image_path};
use crate::herdr::HerdrClient;
use crate::model::{Attachment, ConversationEvent};
use crate::state::StateStore;
use crate::status::extract_status;
use crate::{AppError, AppResult};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use runtime::{RuntimeEvent, UiRuntime};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use terminal::TerminalGuard;

pub fn run_from_env() -> AppResult<()> {
    let source_pane = required_env("HERDR_SIMPLE_PROMPTS_SOURCE_PANE")?;
    let socket = required_env("HERDR_SOCKET_PATH")?;
    let state_root = required_env("HERDR_PLUGIN_STATE_DIR")?;
    let client = HerdrClient::connect(Path::new(&socket))
        .map_err(|error| AppError::new("ui", error.to_string()))?;
    let identity = agent_identity(&client, &source_pane)?;
    let transcript = resolve_transcript(
        identity.kind,
        &identity.session_id,
        &AgentPaths::from_env()?,
    )?;
    let adapter: Box<dyn TranscriptAdapter> = match identity.kind {
        AgentKind::Codex => Box::new(crate::agent::codex::CodexAdapter),
        AgentKind::Claude => Box::new(crate::agent::claude::ClaudeAdapter::default()),
    };
    let mut follower = TranscriptFollower::new(transcript, adapter)?;
    let state_store = StateStore::at(state_root);
    let mut editor = Editor::default();
    let draft = state_store.load_draft(&source_pane)?;
    editor.replace(draft.text);
    let mut app = AppState {
        agent_status: identity.status,
        working_since: identity.status.is_working().then(Instant::now),
        draft_attachments: draft.attachments,
        ..AppState::default()
    };
    apply_follower_events(&mut app, follower.poll_initial(identity.status)?);
    let runtime = UiRuntime::spawn(Path::new(&socket), identity.clone(), follower)?;

    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut local_sequence = 1_u64;

    loop {
        while let Some(event) = runtime.try_recv() {
            apply_runtime_event(
                event,
                &runtime,
                &identity,
                &mut app,
                &mut editor,
                &state_store,
                &source_pane,
            )?;
        }

        terminal.draw(|frame| render::render(frame, &app, &editor))?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                handle_key(
                    key,
                    &mut app,
                    &mut editor,
                    &runtime,
                    &state_store,
                    &source_pane,
                    &mut local_sequence,
                )?;
            }
            Event::Paste(content) => {
                if !app.input_enabled {
                    continue;
                }
                if let Some(path) = staged_image_path(&content) {
                    let attachment = Attachment {
                        id: next_image_id(&mut local_sequence),
                        display: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        native_path: Some(path.clone()),
                    };
                    match runtime.forward_staged_image(attachment.clone(), path) {
                        Ok(()) => app.draft_attachments.push(attachment),
                        Err(error) => app.send_error = Some(error.to_string()),
                    }
                    state_store.save_draft(&source_pane, editor.text(), &app.draft_attachments)?;
                } else {
                    editor.insert_paste(&content);
                    state_store.save_draft(&source_pane, editor.text(), &app.draft_attachments)?;
                }
            }
            Event::Resize(_, _) => {}
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(3)
                }
                MouseEventKind::ScrollDown => {
                    app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(3)
                }
                _ => {}
            },
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    key: KeyEvent,
    app: &mut AppState,
    editor: &mut Editor,
    runtime: &UiRuntime,
    state: &StateStore,
    source_pane: &str,
    local_sequence: &mut u64,
) -> AppResult<()> {
    if !app.input_enabled && !matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
        return Ok(());
    }
    match (key.code, key.modifiers) {
        (KeyCode::Enter, modifiers)
            if modifiers.contains(KeyModifiers::SHIFT)
                || modifiers.contains(KeyModifiers::CONTROL) =>
        {
            editor.newline()
        }
        (KeyCode::Char('j'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            editor.newline()
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            if editor.text().trim().is_empty() && app.draft_attachments.is_empty() {
                return Ok(());
            }
            let text = editor.take_submission();
            app.send_error = None;
            let attachments = app.draft_attachments.clone();
            let local_id = format!("local-{}", *local_sequence);
            *local_sequence += 1;
            app.apply(AppEvent::PromptSubmitted {
                local_id: local_id.clone(),
                text: text.clone(),
                attachments,
                at_ms: now_ms(),
            });
            if let Err(error) = runtime.submit(local_id.clone(), text) {
                app.apply(AppEvent::SendFailed {
                    local_id,
                    reason: error.to_string(),
                });
                editor.replace(app.draft.clone());
                app.send_error = Some(error.to_string());
            }
        }
        (KeyCode::Esc, _) if app.agent_status == AgentStatus::Working => {
            if let Err(error) = runtime.interrupt() {
                app.send_error = Some(error.to_string());
            }
        }
        (KeyCode::Char('v'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            let attachment = Attachment {
                id: next_image_id(local_sequence),
                display: format!("Image #{}", app.draft_attachments.len() + 1),
                native_path: None,
            };
            match runtime.forward_local_image(attachment.clone()) {
                Ok(()) => app.draft_attachments.push(attachment),
                Err(error) => app.send_error = Some(error.to_string()),
            }
        }
        (KeyCode::Backspace, _) => editor.backspace(),
        (KeyCode::Delete, _) => editor.delete(),
        (KeyCode::Left, _) => editor.move_left(),
        (KeyCode::Right, _) => editor.move_right(),
        (KeyCode::Up, _) => editor.move_up(),
        (KeyCode::Down, _) => editor.move_down(),
        (KeyCode::Home, _) => editor.move_home(),
        (KeyCode::End, _) => editor.move_end(),
        (KeyCode::PageUp, _) => app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(5),
        (KeyCode::PageDown, _) => app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(5),
        (KeyCode::Char(character), modifiers)
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            app.send_error = None;
            editor.insert_char(character)
        }
        _ => return Ok(()),
    }
    state.save_draft(source_pane, editor.text(), &app.draft_attachments)?;
    Ok(())
}

fn apply_runtime_event(
    event: RuntimeEvent,
    runtime: &UiRuntime,
    original: &crate::agent::AgentIdentity,
    app: &mut AppState,
    editor: &mut Editor,
    state: &StateStore,
    source_pane: &str,
) -> AppResult<()> {
    match event {
        RuntimeEvent::Transcript(events) => {
            app.transcript_error = None;
            apply_follower_events(app, events);
        }
        RuntimeEvent::TranscriptError(error) => app.transcript_error = Some(error),
        RuntimeEvent::Observation(Ok((current, screen))) => {
            app.connection_error = None;
            app.input_enabled = true;
            let was_working = app.agent_status.is_working();
            app.agent_status = current.status;
            if !was_working && current.status.is_working() {
                app.working_since = Some(Instant::now());
            } else if was_working && !current.status.is_working() {
                app.working_since = None;
                runtime.finalize_pending();
            }
            app.status_line = Some(extract_status(original.kind, &screen, original.cwd.clone()));
        }
        RuntimeEvent::Observation(Err(error)) => {
            app.connection_error = Some(error);
            app.input_enabled = false;
        }
        RuntimeEvent::Submitted { local_id, result } => {
            if let Err(reason) = result {
                app.apply(AppEvent::SendFailed {
                    local_id,
                    reason: reason.clone(),
                });
                editor.replace(app.draft.clone());
                app.send_error = Some(reason);
                state.save_draft(source_pane, editor.text(), &app.draft_attachments)?;
            }
        }
        RuntimeEvent::Interrupted(Err(error)) => app.send_error = Some(error),
        RuntimeEvent::Interrupted(Ok(())) => {}
        RuntimeEvent::ImageForwarded { attachment, result } => {
            if let Err(error) = result {
                app.draft_attachments
                    .retain(|candidate| candidate.id != attachment.id);
                for turn in &mut app.turns {
                    if matches!(turn.delivery, crate::model::Delivery::Optimistic { .. }) {
                        turn.prompt
                            .attachments
                            .retain(|candidate| candidate.id != attachment.id);
                    }
                }
                app.send_error = Some(error);
                state.save_draft(source_pane, editor.text(), &app.draft_attachments)?;
            }
        }
    }
    Ok(())
}

fn apply_follower_events(app: &mut AppState, events: Vec<FollowerEvent>) {
    for event in events {
        match event {
            FollowerEvent::Conversation(ConversationEvent::User(message)) => {
                app.apply(AppEvent::NativeUser(message))
            }
            FollowerEvent::Conversation(ConversationEvent::Final(message)) => {
                app.apply(AppEvent::NativeFinal(message))
            }
            FollowerEvent::Reloaded => app.apply(AppEvent::TranscriptReloaded),
            FollowerEvent::ParseError { line, message } => {
                app.transcript_error = Some(format!("transcript line {line}: {message}"))
            }
        }
    }
}

fn required_env(name: &'static str) -> AppResult<String> {
    std::env::var(name).map_err(|_| AppError::new("ui", format!("{name} is not set")))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn next_image_id(sequence: &mut u64) -> String {
    let id = format!("local-image-{}", *sequence);
    *sequence += 1;
    id
}
