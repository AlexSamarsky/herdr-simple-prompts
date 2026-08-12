pub mod render;
mod runtime;
mod terminal;
pub mod visual_rows;

use crate::agent::follower::{FollowerEvent, TranscriptFollower};
use crate::agent::{
    AgentKind, AgentPaths, AgentStatus, TranscriptAdapter, agent_identity, resolve_transcript,
};
use crate::app::{AppEvent, AppState};
use crate::editor::{Editor, staged_image_path};
use crate::herdr::HerdrClient;
use crate::model::{Attachment, ConversationEvent};
use crate::state::{DraftWriter, StateStore};
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

const DRAFT_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DraftChange {
    None,
    Debounced,
    Immediate,
}

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
    let mut draft = state_store.load_draft(&source_pane)?;
    draft
        .prompt_displays
        .retain(|summary| summary.session_id == identity.session_id);
    let draft_writer = DraftWriter::spawn(state_store.clone(), source_pane.clone());
    editor.replace_snapshot(draft.editor);
    let mut app = AppState {
        session_id: identity.session_id.clone(),
        agent_status: identity.status,
        working_since: identity.status.is_working().then(Instant::now),
        draft_attachments: draft.attachments,
        prompt_displays: draft.prompt_displays,
        ..AppState::default()
    };
    let mut history_cache = render::HistoryRenderCache::default();
    apply_follower_events(
        &mut app,
        follower.poll_initial(identity.status)?,
        &mut history_cache,
    );
    draft_writer.queue_editor(
        editor.snapshot(),
        app.draft_attachments.clone(),
        app.prompt_displays.clone(),
    );
    let runtime = UiRuntime::spawn(Path::new(&socket), identity.clone(), follower)?;

    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    let mut local_sequence = 1_u64;
    let mut draft_dirty = false;
    let mut draft_save_at = Instant::now();

    loop {
        while let Some(event) = runtime.try_recv() {
            let change =
                apply_runtime_event(event, &identity, &mut app, &mut editor, &mut history_cache);
            apply_draft_change(
                change,
                &draft_writer,
                &app,
                &editor,
                &mut draft_dirty,
                &mut draft_save_at,
            );
        }
        if draft_dirty && Instant::now() >= draft_save_at {
            draft_writer.queue_editor(
                editor.snapshot(),
                app.draft_attachments.clone(),
                app.prompt_displays.clone(),
            );
            draft_dirty = false;
        }
        if let Some(error) = draft_writer.take_error() {
            app.send_error = Some(error);
        }

        terminal.draw(|frame| render::render(frame, &app, &editor, &mut history_cache))?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let change = handle_key(
                    key,
                    &mut app,
                    &mut editor,
                    &runtime,
                    &mut local_sequence,
                    &mut history_cache,
                )?;
                apply_draft_change(
                    change,
                    &draft_writer,
                    &app,
                    &editor,
                    &mut draft_dirty,
                    &mut draft_save_at,
                );
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
                        Ok(()) => app.pending_attachments.push(attachment),
                        Err(error) => app.send_error = Some(error.to_string()),
                    }
                } else {
                    editor.insert_paste(&content);
                    apply_draft_change(
                        DraftChange::Debounced,
                        &draft_writer,
                        &app,
                        &editor,
                        &mut draft_dirty,
                        &mut draft_save_at,
                    );
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

fn handle_key(
    key: KeyEvent,
    app: &mut AppState,
    editor: &mut Editor,
    runtime: &UiRuntime,
    local_sequence: &mut u64,
    history_cache: &mut render::HistoryRenderCache,
) -> AppResult<DraftChange> {
    if !app.input_enabled && !matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
        return Ok(DraftChange::None);
    }
    let change = match (key.code, key.modifiers) {
        (KeyCode::Enter, modifiers)
            if modifiers.contains(KeyModifiers::SHIFT)
                || modifiers.contains(KeyModifiers::CONTROL) =>
        {
            editor.newline();
            DraftChange::Debounced
        }
        (KeyCode::Char('j'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            editor.newline();
            DraftChange::Debounced
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            if !app.pending_attachments.is_empty() {
                app.send_error = Some("wait for image attachment verification".to_owned());
                return Ok(DraftChange::None);
            }
            if editor.submission_text().trim().is_empty() && app.draft_attachments.is_empty() {
                return Ok(DraftChange::None);
            }
            let submission = editor.take_editor_submission();
            let complete_text = submission.complete_text.clone();
            app.send_error = None;
            let attachments = app.draft_attachments.clone();
            let local_id = format!("local-{}", *local_sequence);
            *local_sequence += 1;
            app.apply(AppEvent::PromptSubmitted {
                local_id: local_id.clone(),
                submission,
                attachments,
                at_ms: now_ms(),
            });
            history_cache.invalidate();
            if let Err(error) = runtime.submit(local_id.clone(), complete_text) {
                app.apply(AppEvent::SendFailed {
                    local_id,
                    reason: error.to_string(),
                });
                history_cache.invalidate();
                editor.replace_snapshot(app.draft.clone());
                app.send_error = Some(error.to_string());
            }
            DraftChange::Immediate
        }
        (KeyCode::Esc, _) if app.agent_status == AgentStatus::Working => {
            if let Err(error) = runtime.interrupt() {
                app.send_error = Some(error.to_string());
            }
            DraftChange::None
        }
        (KeyCode::Char('v'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            let attachment = Attachment {
                id: next_image_id(local_sequence),
                display: format!("Image #{}", app.draft_attachments.len() + 1),
                native_path: None,
            };
            match runtime.forward_local_image(attachment.clone()) {
                Ok(()) => app.pending_attachments.push(attachment),
                Err(error) => app.send_error = Some(error.to_string()),
            }
            DraftChange::None
        }
        (KeyCode::Backspace, _) => {
            editor.backspace();
            DraftChange::Debounced
        }
        (KeyCode::Delete, _) => {
            editor.delete();
            DraftChange::Debounced
        }
        (KeyCode::Left, _) => {
            editor.move_left();
            DraftChange::None
        }
        (KeyCode::Right, _) => {
            editor.move_right();
            DraftChange::None
        }
        (KeyCode::Up, _) => {
            editor.move_up();
            DraftChange::None
        }
        (KeyCode::Down, _) => {
            editor.move_down();
            DraftChange::None
        }
        (KeyCode::Home, _) => {
            editor.move_home();
            DraftChange::None
        }
        (KeyCode::End, _) => {
            editor.move_end();
            DraftChange::None
        }
        (KeyCode::PageUp, _) => {
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(5);
            DraftChange::None
        }
        (KeyCode::PageDown, _) => {
            app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(5);
            DraftChange::None
        }
        (KeyCode::Char(character), modifiers)
            if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
        {
            app.send_error = None;
            editor.insert_char(character);
            DraftChange::Debounced
        }
        _ => return Ok(DraftChange::None),
    };
    Ok(change)
}

fn apply_runtime_event(
    event: RuntimeEvent,
    original: &crate::agent::AgentIdentity,
    app: &mut AppState,
    editor: &mut Editor,
    history_cache: &mut render::HistoryRenderCache,
) -> DraftChange {
    match event {
        RuntimeEvent::Transcript(events) => {
            app.transcript_error = None;
            apply_follower_events(app, events, history_cache)
        }
        RuntimeEvent::TranscriptError(error) => {
            app.transcript_error = Some(error);
            DraftChange::None
        }
        RuntimeEvent::Observation(Ok((current, screen))) => {
            app.connection_error = None;
            app.input_enabled = true;
            let was_working = app.agent_status.is_working();
            app.agent_status = current.status;
            if !was_working && current.status.is_working() {
                app.working_since = Some(Instant::now());
            } else if was_working && !current.status.is_working() {
                app.working_since = None;
            }
            app.status_line = Some(extract_status(original.kind, &screen, original.cwd.clone()));
            DraftChange::None
        }
        RuntimeEvent::Observation(Err(error)) => {
            app.connection_error = Some(error);
            app.input_enabled = false;
            DraftChange::None
        }
        RuntimeEvent::Submitted { local_id, result } => {
            if let Err(reason) = result {
                app.apply(AppEvent::SendFailed {
                    local_id,
                    reason: reason.clone(),
                });
                history_cache.invalidate();
                editor.replace_snapshot(app.draft.clone());
                app.send_error = Some(reason);
                DraftChange::Immediate
            } else {
                DraftChange::None
            }
        }
        RuntimeEvent::Interrupted(Err(error)) => {
            app.send_error = Some(error);
            DraftChange::None
        }
        RuntimeEvent::Interrupted(Ok(())) => DraftChange::None,
        RuntimeEvent::ImageForwarded { attachment, result } => {
            app.pending_attachments
                .retain(|candidate| candidate.id != attachment.id);
            match result {
                Ok(()) => {
                    app.draft_attachments.push(attachment);
                    DraftChange::Immediate
                }
                Err(error) => {
                    app.send_error = Some(error);
                    DraftChange::None
                }
            }
        }
    }
}

fn apply_draft_change(
    change: DraftChange,
    writer: &DraftWriter,
    app: &AppState,
    editor: &Editor,
    dirty: &mut bool,
    save_at: &mut Instant,
) {
    match change {
        DraftChange::None => {}
        DraftChange::Debounced => {
            *dirty = true;
            *save_at = Instant::now() + DRAFT_DEBOUNCE;
        }
        DraftChange::Immediate => {
            writer.queue_editor(
                editor.snapshot(),
                app.draft_attachments.clone(),
                app.prompt_displays.clone(),
            );
            *dirty = false;
        }
    }
}

fn apply_follower_events(
    app: &mut AppState,
    events: Vec<FollowerEvent>,
    history_cache: &mut render::HistoryRenderCache,
) -> DraftChange {
    let prompt_displays_before = app.prompt_displays.clone();
    let replayed = events
        .iter()
        .any(|event| matches!(event, FollowerEvent::Reloaded));
    for event in events {
        match event {
            FollowerEvent::Conversation(ConversationEvent::User(message)) => {
                app.apply(AppEvent::NativeUser(message));
                history_cache.invalidate();
            }
            FollowerEvent::Conversation(ConversationEvent::Final(message)) => {
                app.apply(AppEvent::NativeFinal(message));
                history_cache.invalidate();
            }
            FollowerEvent::Reloaded => {
                app.apply(AppEvent::TranscriptReloaded);
                history_cache.invalidate();
            }
            FollowerEvent::ParseError { line, message } => {
                app.transcript_error = Some(format!("transcript line {line}: {message}"))
            }
        }
    }
    if replayed {
        app.apply(AppEvent::TranscriptReplayComplete);
    }
    if app.prompt_displays == prompt_displays_before {
        DraftChange::None
    } else {
        DraftChange::Immediate
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
