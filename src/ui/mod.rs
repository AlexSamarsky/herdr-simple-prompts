pub mod render;
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
use crate::transport::AgentTransport;
use crate::{AppError, AppResult};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
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
    let transport = AgentTransport::new(client, identity.clone());
    let state_store = StateStore::at(state_root);
    let mut editor = Editor::default();
    editor.replace(state_store.load_draft(&source_pane)?);
    let mut app = AppState {
        agent_status: identity.status,
        working_since: identity.status.is_working().then(Instant::now),
        ..AppState::default()
    };
    apply_follower_events(&mut app, follower.poll()?);

    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut next_transcript_poll = Instant::now();
    let mut next_status_poll = Instant::now();
    let mut local_sequence = 1_u64;

    loop {
        let now = Instant::now();
        if now >= next_transcript_poll {
            match follower.poll() {
                Ok(events) => apply_follower_events(&mut app, events),
                Err(error) => app.error = Some(error.to_string()),
            }
            next_transcript_poll = now + Duration::from_millis(100);
        }
        if now >= next_status_poll {
            match transport.refresh_identity() {
                Ok(current) => {
                    let was_working = app.agent_status.is_working();
                    app.agent_status = current.status;
                    if !was_working && current.status.is_working() {
                        app.working_since = Some(Instant::now());
                    } else if was_working && !current.status.is_working() {
                        app.working_since = None;
                        if let Some(event) = follower.finalize_pending() {
                            apply_follower_events(&mut app, vec![event]);
                        }
                    }
                    match transport.visible_source(8) {
                        Ok(screen) => {
                            app.status_line =
                                Some(extract_status(identity.kind, &screen, identity.cwd.clone()));
                            app.error = None;
                        }
                        Err(error) => app.error = Some(error.to_string()),
                    }
                }
                Err(error) => app.error = Some(error.to_string()),
            }
            next_status_poll = now + Duration::from_millis(200);
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
                    &transport,
                    &state_store,
                    &source_pane,
                    &mut terminal,
                    &mut local_sequence,
                )?;
            }
            Event::Paste(content) => {
                if let Some(path) = staged_image_path(&content) {
                    match transport.forward_staged_image(&path) {
                        Ok(()) => app.draft_attachments.push(Attachment {
                            id: format!("local-image-{}", app.draft_attachments.len() + 1),
                            display: path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            native_path: Some(path),
                        }),
                        Err(error) => app.error = Some(error.to_string()),
                    }
                } else {
                    editor.insert_paste(&content);
                    state_store.save_draft(&source_pane, editor.text())?;
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
    transport: &AgentTransport,
    state: &StateStore,
    source_pane: &str,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    local_sequence: &mut u64,
) -> AppResult<()> {
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
            let attachments = app.draft_attachments.clone();
            let local_id = format!("local-{}", *local_sequence);
            *local_sequence += 1;
            app.apply(AppEvent::PromptSubmitted {
                local_id: local_id.clone(),
                text: text.clone(),
                attachments,
                at_ms: now_ms(),
            });
            terminal.draw(|frame| render::render(frame, app, editor))?;
            if let Err(error) = transport.submit(&text) {
                app.apply(AppEvent::SendFailed {
                    local_id,
                    reason: error.to_string(),
                });
                editor.replace(app.draft.clone());
                app.error = Some(error.to_string());
            }
        }
        (KeyCode::Esc, _) if app.agent_status == AgentStatus::Working => {
            if let Err(error) = transport.interrupt() {
                app.error = Some(error.to_string());
            }
        }
        (KeyCode::Char('v'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            match transport.forward_local_image_paste() {
                Ok(()) => app.draft_attachments.push(Attachment {
                    id: format!("local-image-{}", app.draft_attachments.len() + 1),
                    display: format!("Image #{}", app.draft_attachments.len() + 1),
                    native_path: None,
                }),
                Err(error) => app.error = Some(error.to_string()),
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
            editor.insert_char(character)
        }
        _ => return Ok(()),
    }
    state.save_draft(source_pane, editor.text())?;
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
            FollowerEvent::Reloaded => {}
            FollowerEvent::ParseError { line, message } => {
                app.error = Some(format!("transcript line {line}: {message}"))
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
