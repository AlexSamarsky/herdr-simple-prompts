pub mod interaction;
pub mod render;
mod runtime;
mod terminal;
pub mod visual_rows;

use crate::agent::follower::{FollowerEvent, TranscriptFollower};
use crate::agent::{
    AgentKind, AgentPaths, AgentStatus, TranscriptAdapter, agent_identity, resolve_transcript,
};
use crate::ansi::sanitize_ansi;
use crate::app::{AppEvent, AppState};
use crate::composer::{ComposerAccess, NativeComposerState, native_composer_text};
use crate::editor::{Editor, staged_image_path};
use crate::herdr::HerdrClient;
use crate::history::HistoryWriter;
use crate::model::{Attachment, ConversationEvent};
use crate::state::{DraftWriter, StateStore};
use crate::status::extract_status;
use crate::{AppError, AppResult};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use interaction::{map_interaction_key, map_interaction_paste};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use runtime::{RuntimeEvent, UiRuntime};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use terminal::TerminalGuard;

const DRAFT_DEBOUNCE: Duration = Duration::from_millis(250);
const ADOPT_ATTEMPTS: usize = 8;
const ADOPT_RETRY_DELAY: Duration = Duration::from_millis(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CapturePolicy {
    NewestFinalOnly,
    AllFinals,
}

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
    let state_store = StateStore::at(state_root);
    let identity = state_store.with_lifecycle_lock(|| {
        state_store.validate_saved_namespaces(&client, now_ms())?;
        let identity = agent_identity(&client, &source_pane)?;
        state_store.bind_verified_namespace(&source_pane, &identity.session_id, now_ms())?;
        Ok(identity)
    })?;
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
    let history_journal = state_store.history_journal(&source_pane, &identity.session_id)?;
    let saved_history = history_journal.load()?;
    let mut history_writer = Some(HistoryWriter::spawn(history_journal));
    let mut editor = Editor::default();
    let mut draft = state_store.load_draft(&source_pane)?;
    draft
        .prompt_displays
        .retain(|summary| summary.session_id == identity.session_id);
    let mut draft_writer = Some(DraftWriter::spawn(
        state_store.clone(),
        source_pane.clone(),
        Some(identity.session_id.clone()),
    ));
    editor.replace_snapshot(draft.editor);
    let adopted = editor
        .is_empty()
        .then(|| {
            adopt_native_draft(
                identity.kind,
                || {
                    client
                        .pane_read_visible_ansi(&source_pane, 200)
                        .map_err(|error| AppError::new("native draft", error.to_string()))
                },
                || {
                    client
                        .agent_send_keys(&source_pane, &["ctrl+e", "ctrl+u"])
                        .map(|_| ())
                        .map_err(|error| AppError::new("native draft", error.to_string()))
                },
                ADOPT_ATTEMPTS,
                ADOPT_RETRY_DELAY,
            )
        })
        .transpose();
    let adopt_notice = match adopted {
        Ok(Some(Some(adopted))) => {
            editor.replace(adopted.text);
            (!adopted.cleared)
                .then(|| "native composer still holds a copy of the adopted draft".to_owned())
        }
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };

    let mut app = AppState {
        session_id: identity.session_id.clone(),
        agent_status: identity.status,
        native_composer: NativeComposerState::Unknown,
        working_since: identity.status.is_working().then(Instant::now),
        draft_attachments: draft.attachments,
        prompt_displays: draft.prompt_displays,
        ..AppState::default()
    };
    app.hydrate_visible_history(saved_history);
    app.background_error = adopt_notice;
    let mut history_cache = render::HistoryRenderCache::default();
    let initial_events = follower.poll_initial(identity.status)?;
    let runtime = UiRuntime::spawn(Path::new(&socket), identity.clone(), follower)?;
    apply_follower_events_with_policy(
        &mut app,
        initial_events,
        &mut history_cache,
        &runtime,
        CapturePolicy::NewestFinalOnly,
    );
    queue_history_upserts(&mut app, history_writer.as_ref());
    draft_writer
        .as_ref()
        .expect("draft writer exists")
        .queue_editor(
            editor.snapshot(),
            app.draft_attachments.clone(),
            app.prompt_displays.clone(),
        );
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
            if matches!(&event, RuntimeEvent::SourcePaneClosed) {
                if let Some(writer) = history_writer.take() {
                    writer.cancel();
                }
                drop(draft_writer.take());
                if let Err(error) =
                    state_store.with_lifecycle_lock(|| state_store.remove_pane_state(&source_pane))
                {
                    app.transcript_error = Some(format!("source cleanup: {error}"));
                }
                app.source_pane_closed();
                draft_dirty = false;
                continue;
            }
            let change = apply_runtime_event(
                event,
                &identity,
                &mut app,
                &mut editor,
                &mut history_cache,
                &runtime,
            );
            queue_history_upserts(&mut app, history_writer.as_ref());
            if let Some(writer) = draft_writer.as_ref() {
                apply_draft_change(
                    change,
                    writer,
                    &app,
                    &editor,
                    &mut draft_dirty,
                    &mut draft_save_at,
                );
            }
        }
        if draft_dirty && Instant::now() >= draft_save_at {
            if let Some(writer) = draft_writer.as_ref() {
                writer.queue_editor(
                    editor.snapshot(),
                    app.draft_attachments.clone(),
                    app.prompt_displays.clone(),
                );
            }
            draft_dirty = false;
        }
        if let Some(error) = draft_writer.as_ref().and_then(DraftWriter::take_error) {
            app.background_error = Some(format!("draft: {error}"));
        }
        if let Some(error) = history_writer.as_ref().and_then(HistoryWriter::take_error) {
            app.background_error = Some(format!("history: {error}"));
        }

        render::draw_terminal(&mut terminal, &app, &editor, &mut history_cache)?;
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
                if let Some(writer) = draft_writer.as_ref() {
                    apply_draft_change(
                        change,
                        writer,
                        &app,
                        &editor,
                        &mut draft_dirty,
                        &mut draft_save_at,
                    );
                }
            }
            Event::Paste(content) => {
                if handle_blocked_paste(&content, &mut app, &runtime) {
                    continue;
                }
                let change = handle_ordinary_paste(
                    &content,
                    &mut app,
                    &mut editor,
                    &runtime,
                    &mut local_sequence,
                );
                if let Some(writer) = draft_writer.as_ref() {
                    apply_draft_change(
                        change,
                        writer,
                        &app,
                        &editor,
                        &mut draft_dirty,
                        &mut draft_save_at,
                    );
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn handle_blocked_paste(content: &str, app: &mut AppState, runtime: &UiRuntime) -> bool {
    if app.agent_status != AgentStatus::Blocked {
        return false;
    }
    if let Err(error) = runtime.forward_interaction(map_interaction_paste(content)) {
        app.interaction_error = Some(error.to_string());
    }
    true
}

fn handle_ordinary_paste(
    content: &str,
    app: &mut AppState,
    editor: &mut Editor,
    runtime: &UiRuntime,
    local_sequence: &mut u64,
) -> DraftChange {
    if !ordinary_input_allowed(app) {
        return DraftChange::None;
    }
    if let Some(path) = staged_image_path(content) {
        let attachment = Attachment {
            id: next_image_id(local_sequence),
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
        DraftChange::None
    } else {
        editor.insert_paste(content);
        DraftChange::Debounced
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
    if app.agent_status == AgentStatus::Blocked {
        if let Some(input) = map_interaction_key(key)
            && let Err(error) = runtime.forward_interaction(input)
        {
            app.interaction_error = Some(error.to_string());
        }
        return Ok(DraftChange::None);
    }
    match key.code {
        KeyCode::PageUp => {
            history_cache.scroll_up(5);
            app.scroll_from_bottom = history_cache.scroll_from_bottom();
            return Ok(DraftChange::None);
        }
        KeyCode::PageDown => {
            history_cache.scroll_down(5);
            app.scroll_from_bottom = history_cache.scroll_from_bottom();
            return Ok(DraftChange::None);
        }
        KeyCode::Up if scrolls_history(app, editor) => {
            history_cache.scroll_up(1);
            app.scroll_from_bottom = history_cache.scroll_from_bottom();
            return Ok(DraftChange::None);
        }
        KeyCode::Down if scrolls_history(app, editor) => {
            history_cache.scroll_down(1);
            app.scroll_from_bottom = history_cache.scroll_from_bottom();
            return Ok(DraftChange::None);
        }
        KeyCode::Esc if app.agent_status == AgentStatus::Working => {
            if let Err(error) = runtime.interrupt() {
                app.send_error = Some(error.to_string());
            }
            return Ok(DraftChange::None);
        }
        _ => {}
    }
    if !ordinary_input_allowed(app) {
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
            let expected_attachments = attachments.len();
            let local_id = format!("local-{}", *local_sequence);
            *local_sequence += 1;
            app.apply(AppEvent::PromptSubmitted {
                local_id: local_id.clone(),
                submission,
                attachments,
                at_ms: now_ms(),
            });
            history_cache.invalidate();
            if let Err(error) =
                runtime.submit(local_id.clone(), complete_text, expected_attachments)
            {
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
        (KeyCode::Left, modifiers) if modifiers.contains(KeyModifiers::SUPER) => {
            editor.move_home();
            DraftChange::None
        }
        (KeyCode::Right, modifiers) if modifiers.contains(KeyModifiers::SUPER) => {
            editor.move_end();
            DraftChange::None
        }
        (KeyCode::Up, modifiers) if modifiers.contains(KeyModifiers::SUPER) => {
            editor.move_document_start();
            DraftChange::None
        }
        (KeyCode::Down, modifiers) if modifiers.contains(KeyModifiers::SUPER) => {
            editor.move_document_end();
            DraftChange::None
        }
        // Killing to a line end. macOS sends `^U` for command+backspace, which
        // is what actually arrives; the super arms cover terminals configured
        // to forward the modifier itself.
        (KeyCode::Backspace, modifiers) if modifiers.contains(KeyModifiers::SUPER) => {
            editor.delete_to_line_start();
            DraftChange::Debounced
        }
        (KeyCode::Char('u'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            editor.delete_to_line_start();
            DraftChange::Debounced
        }
        (KeyCode::Delete, modifiers) if modifiers.contains(KeyModifiers::SUPER) => {
            editor.delete_to_line_end();
            DraftChange::Debounced
        }
        (KeyCode::Char('k'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            editor.delete_to_line_end();
            DraftChange::Debounced
        }
        // Word editing. Terminals disagree about what the option key sends:
        // some emit a modified arrow, others the readline escapes (`alt+b`,
        // `alt+f`, `alt+d`), so both spellings are accepted.
        (KeyCode::Backspace, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            editor.delete_word_left();
            DraftChange::Debounced
        }
        (KeyCode::Char('w'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            editor.delete_word_left();
            DraftChange::Debounced
        }
        (KeyCode::Delete, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            editor.delete_word_right();
            DraftChange::Debounced
        }
        (KeyCode::Char('d'), modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            editor.delete_word_right();
            DraftChange::Debounced
        }
        (KeyCode::Left, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            editor.move_word_left();
            DraftChange::None
        }
        (KeyCode::Char('b'), modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            editor.move_word_left();
            DraftChange::None
        }
        (KeyCode::Right, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            editor.move_word_right();
            DraftChange::None
        }
        (KeyCode::Char('f'), modifiers) if modifiers.contains(KeyModifiers::ALT) => {
            editor.move_word_right();
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
        // Line and document navigation. macOS never delivers the command key
        // to a terminal application — the multiplexer has no name for that
        // modifier at all — so the readline bindings are the ones that reach
        // us, and the super arms fire only where a terminal is configured to
        // send the modifier through.
        (KeyCode::Char('a'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            editor.move_home();
            DraftChange::None
        }
        (KeyCode::Char('e'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            editor.move_end();
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct AdoptedDraft {
    text: String,
    cleared: bool,
}

/// Takes over whatever the native composer is holding.
///
/// Opening the overlay over a half-written prompt used to mean switching back
/// to finish it, because the overlay refuses to send while the native composer
/// is occupied. Move the text here instead — and clear it there, since leaving
/// a copy behind would splice it onto the next prompt.
///
/// The text is returned even when clearing fails, so a draft is never lost to a
/// half-finished takeover; the caller says so and the guard stays on.
fn adopt_native_draft(
    kind: crate::agent::AgentKind,
    mut read: impl FnMut() -> AppResult<String>,
    mut clear: impl FnMut() -> AppResult<()>,
    attempts: usize,
    retry_delay: Duration,
) -> AppResult<Option<AdoptedDraft>> {
    let Some(text) = native_composer_text(kind, &sanitize_ansi(&read()?)) else {
        return Ok(None);
    };
    for attempt in 0..attempts {
        clear()?;
        if attempt + 1 < attempts {
            std::thread::sleep(retry_delay);
        }
        if native_composer_text(kind, &sanitize_ansi(&read()?)).is_none() {
            return Ok(Some(AdoptedDraft {
                text,
                cleared: true,
            }));
        }
    }
    Ok(Some(AdoptedDraft {
        text,
        cleared: false,
    }))
}

fn ordinary_input_allowed(app: &AppState) -> bool {
    app.input_enabled && app.composer_access() == ComposerAccess::Ready
}

/// The wheel arrives as arrow keys, so the arrows drive the history whenever
/// the composer has no text for a cursor to move through — and always while the
/// composer is guarded, so navigation never depends on the native pane.
fn scrolls_history(app: &AppState, editor: &Editor) -> bool {
    !ordinary_input_allowed(app) || editor.display_text().is_empty()
}

fn apply_runtime_event(
    event: RuntimeEvent,
    original: &crate::agent::AgentIdentity,
    app: &mut AppState,
    editor: &mut Editor,
    history_cache: &mut render::HistoryRenderCache,
    runtime: &UiRuntime,
) -> DraftChange {
    match event {
        RuntimeEvent::Transcript(events) => {
            app.transcript_error = None;
            apply_follower_events_with_policy(
                app,
                events,
                history_cache,
                runtime,
                CapturePolicy::AllFinals,
            )
        }
        RuntimeEvent::TranscriptError(error) => {
            app.transcript_error = Some(error);
            DraftChange::None
        }
        RuntimeEvent::Observation(Ok(observation)) => {
            if app.source_pane_closed {
                return DraftChange::None;
            }
            app.connection_error = None;
            app.input_enabled = true;
            app.native_composer = observation.native_composer;
            let current = observation.identity;
            let was_working = app.agent_status.is_working();
            if !was_working && current.status.is_working() {
                app.working_since = Some(Instant::now());
            } else if was_working && !current.status.is_working() {
                app.working_since = None;
            }
            app.update_blocked_surface(current.status, observation.blocked_surface);
            app.status_line = Some(extract_status(
                original.kind,
                &observation.status_text,
                original.cwd.clone(),
            ));
            DraftChange::None
        }
        RuntimeEvent::Observation(Err(error)) => {
            if app.source_pane_closed {
                return DraftChange::None;
            }
            app.connection_error = Some(error);
            app.input_enabled = false;
            app.native_composer = NativeComposerState::Unknown;
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
        RuntimeEvent::InteractionForwarded(result) => {
            app.apply_interaction_result(result);
            DraftChange::None
        }
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
        RuntimeEvent::FinalPresentation {
            stable_id,
            text_fingerprint,
            presentation,
        } => {
            app.apply(AppEvent::FinalPresentation {
                stable_id,
                text_fingerprint,
                presentation,
            });
            history_cache.invalidate();
            DraftChange::None
        }
        RuntimeEvent::CaptureDiagnostic(error) => {
            app.transcript_error = Some(format!("final style capture: {error}"));
            DraftChange::None
        }
        RuntimeEvent::SourcePaneClosed => {
            app.source_pane_closed();
            DraftChange::None
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

fn queue_history_upserts(app: &mut AppState, writer: Option<&HistoryWriter>) {
    for record in app.drain_history_upserts() {
        if let Some(writer) = writer {
            writer.queue(record);
        }
    }
}

fn apply_follower_events_with_policy(
    app: &mut AppState,
    events: Vec<FollowerEvent>,
    history_cache: &mut render::HistoryRenderCache,
    runtime: &UiRuntime,
    capture_policy: CapturePolicy,
) -> DraftChange {
    let prompt_displays_before = app.prompt_displays.clone();
    let replayed = events
        .iter()
        .any(|event| matches!(event, FollowerEvent::Reloaded))
        || capture_policy == CapturePolicy::NewestFinalOnly;
    if replayed {
        app.apply(AppEvent::TranscriptReloaded);
        history_cache.invalidate();
    }
    let newest_final = (capture_policy == CapturePolicy::NewestFinalOnly)
        .then(|| {
            events.iter().rposition(|event| {
                matches!(
                    event,
                    FollowerEvent::Conversation(ConversationEvent::Final(_))
                )
            })
        })
        .flatten();
    for (event_index, event) in events.into_iter().enumerate() {
        match event {
            FollowerEvent::Conversation(ConversationEvent::User(message)) => {
                app.apply(AppEvent::NativeUser(message));
                history_cache.invalidate();
            }
            FollowerEvent::Conversation(ConversationEvent::Final(message)) => {
                let stable_id = message.stable_id.clone();
                let canonical_text = message.text.clone();
                app.apply(AppEvent::NativeFinal(message));
                history_cache.invalidate();
                let should_capture =
                    capture_policy == CapturePolicy::AllFinals || newest_final == Some(event_index);
                if should_capture
                    && let Err(error) = runtime.capture_final(stable_id, canonical_text)
                {
                    app.transcript_error = Some(error.to_string());
                }
            }
            FollowerEvent::Reloaded => {}
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

#[cfg(test)]
mod tests {
    use super::{
        CapturePolicy, apply_follower_events_with_policy, apply_runtime_event,
        handle_blocked_paste, handle_key, handle_ordinary_paste, render, runtime,
    };
    use crate::agent::follower::FollowerEvent;
    use crate::agent::{AgentIdentity, AgentKind, AgentStatus};
    use crate::app::AppState;
    use crate::composer::NativeComposerState;
    use crate::editor::Editor;
    use crate::history::{PersistedPresentation, VisibleHistoryRecord, VisibleRole};
    use crate::model::{Attachment, ConversationEvent, Message};
    use crate::paste::fingerprint;
    use crate::ui::interaction::InteractionInput;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn final_event(index: usize) -> FollowerEvent {
        FollowerEvent::Conversation(ConversationEvent::Final(Message::final_text(
            format!("final-{index}"),
            format!("answer-{index}"),
            Some(index as u64),
        )))
    }

    #[test]
    fn initial_replay_with_more_than_capture_capacity_enqueues_only_newest_final() {
        let (runtime, captures) = runtime::capture_test_runtime(8);
        let mut app = AppState::default();
        let mut cache = render::HistoryRenderCache::default();
        let mut events = Vec::new();
        for index in 0..10 {
            events.push(FollowerEvent::Conversation(ConversationEvent::User(
                Message::text(format!("prompt-{index}"), format!("prompt {index}"), None),
            )));
            events.push(final_event(index));
        }

        apply_follower_events_with_policy(
            &mut app,
            events,
            &mut cache,
            &runtime,
            CapturePolicy::NewestFinalOnly,
        );

        let captured: Vec<_> = captures.try_iter().collect();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].stable_id, "final-9");
        assert_eq!(app.turns.len(), 10);
    }

    /// Style capture is an enhancement layered over an answer that already
    /// renders, so a saturated queue must not surface as an error line.
    #[test]
    fn saturated_capture_queue_is_not_reported_as_an_error() {
        let (runtime, captures) = runtime::capture_test_runtime(2);
        let mut app = AppState::default();
        let mut cache = render::HistoryRenderCache::default();
        let mut events = Vec::new();
        for index in 0..5 {
            events.push(FollowerEvent::Conversation(ConversationEvent::User(
                Message::text(format!("prompt-{index}"), format!("prompt {index}"), None),
            )));
            events.push(final_event(index));
        }

        apply_follower_events_with_policy(
            &mut app,
            events,
            &mut cache,
            &runtime,
            CapturePolicy::AllFinals,
        );

        assert_eq!(captures.try_iter().count(), 2);
        assert_eq!(app.transcript_error, None);
        assert_eq!(app.turns.len(), 5);
    }

    #[test]
    fn live_batch_enqueues_each_new_final() {
        let (runtime, captures) = runtime::capture_test_runtime(8);
        let mut app = AppState::default();
        let mut cache = render::HistoryRenderCache::default();
        let events = vec![
            FollowerEvent::Conversation(ConversationEvent::User(Message::text(
                "prompt-1", "prompt 1", None,
            ))),
            final_event(1),
            FollowerEvent::Conversation(ConversationEvent::User(Message::text(
                "prompt-2", "prompt 2", None,
            ))),
            final_event(2),
        ];

        apply_follower_events_with_policy(
            &mut app,
            events,
            &mut cache,
            &runtime,
            CapturePolicy::AllFinals,
        );

        let captured: Vec<_> = captures.try_iter().collect();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].stable_id, "final-1");
        assert_eq!(captured[1].stable_id, "final-2");
    }

    #[test]
    fn initial_batch_reconciles_hydrated_history_in_native_replay_order() {
        let (runtime, _captures) = runtime::capture_test_runtime(8);
        let mut app = AppState::default();
        app.hydrate_visible_history(vec![
            VisibleHistoryRecord {
                version: 2,
                role: VisibleRole::Prompt,
                stable_id: "prompt-2".into(),
                turn_id: "prompt-2".into(),
                order: 1,
                text: "saved prompt 2".into(),
                attachments: Vec::new(),
                timestamp_ms: Some(1),
                text_fingerprint: fingerprint("saved prompt 2"),
                presentation: PersistedPresentation::Plain,
                rendered_text: None,
                rendered_text_fingerprint: None,
            },
            VisibleHistoryRecord {
                version: 2,
                role: VisibleRole::Prompt,
                stable_id: "saved-missing".into(),
                turn_id: "saved-missing".into(),
                order: 2,
                text: "retain me".into(),
                attachments: Vec::new(),
                timestamp_ms: Some(2),
                text_fingerprint: fingerprint("retain me"),
                presentation: PersistedPresentation::Plain,
                rendered_text: None,
                rendered_text_fingerprint: None,
            },
        ]);
        let mut cache = render::HistoryRenderCache::default();
        let events = vec![
            FollowerEvent::Conversation(ConversationEvent::User(Message::text(
                "prompt-1",
                "native prompt 1",
                Some(10),
            ))),
            final_event(1),
            FollowerEvent::Conversation(ConversationEvent::User(Message::text(
                "prompt-2",
                "native prompt 2",
                Some(20),
            ))),
            final_event(2),
        ];

        apply_follower_events_with_policy(
            &mut app,
            events,
            &mut cache,
            &runtime,
            CapturePolicy::NewestFinalOnly,
        );

        assert_eq!(
            app.turns
                .iter()
                .map(|turn| turn.prompt.stable_id.as_str())
                .collect::<Vec<_>>(),
            ["prompt-1", "prompt-2", "saved-missing"]
        );
        assert_eq!(
            app.turns[0].final_answer.as_ref().unwrap().stable_id,
            "final-1"
        );
        assert_eq!(
            app.turns[1].final_answer.as_ref().unwrap().stable_id,
            "final-2"
        );
        assert!(app.turns[2].final_answer.is_none());
        assert_eq!(app.replay_insert_at, None);
    }

    #[test]
    fn blocked_key_routes_before_composer_and_preserves_editor_and_history_state() {
        let (runtime, actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            agent_status: AgentStatus::Blocked,
            scroll_from_bottom: 7,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        editor.insert_paste("unchanged draft");
        let before = editor.snapshot();
        let mut sequence = 9;
        let mut cache = render::HistoryRenderCache::default();

        let change = handle_key(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &mut app,
            &mut editor,
            &runtime,
            &mut sequence,
            &mut cache,
        )
        .unwrap();

        assert_eq!(change, super::DraftChange::None);
        assert_eq!(editor.snapshot(), before);
        assert_eq!(app.scroll_from_bottom, 7);
        assert_eq!(sequence, 9);
        assert!(matches!(
            actions.try_recv().unwrap(),
            runtime::ActionCommand::Interaction(InteractionInput::Key("down"))
        ));
        assert!(actions.try_recv().is_err());
    }

    #[test]
    fn blocked_paste_routes_as_one_text_action_without_composer_work() {
        let (runtime, actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            agent_status: AgentStatus::Blocked,
            ..AppState::default()
        };
        let content = "large body\n".repeat(1_000);

        assert!(handle_blocked_paste(&content, &mut app, &runtime));
        assert!(matches!(
            actions.try_recv().unwrap(),
            runtime::ActionCommand::Interaction(InteractionInput::Text(text)) if text == content
        ));
        assert!(app.pending_attachments.is_empty());
        assert!(app.draft_attachments.is_empty());
    }

    /// Word editing has to answer both spellings of the option key: a modified
    /// arrow, and the readline escapes that other terminals send instead.
    #[test]
    fn word_hotkeys_edit_the_composer_in_both_spellings() {
        for (delete_left, delete_right, left, right) in [
            (
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Delete, KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Left, KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Right, KeyModifiers::ALT),
            ),
            (
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
            ),
        ] {
            let (runtime, _actions) = runtime::interaction_test_runtime(1);
            let mut app = AppState {
                native_composer: NativeComposerState::Clear,
                ..AppState::default()
            };
            let mut editor = Editor::default();
            editor.insert_paste("alpha beta gamma");
            let mut sequence = 1;
            let mut cache = render::HistoryRenderCache::default();
            let mut press = |key, app: &mut AppState, editor: &mut Editor, cache: &mut _| {
                handle_key(key, app, editor, &runtime, &mut sequence, cache).unwrap()
            };

            assert_eq!(
                press(delete_left, &mut app, &mut editor, &mut cache),
                super::DraftChange::Debounced
            );
            assert_eq!(editor.submission_text(), "alpha beta ");

            assert_eq!(
                press(left, &mut app, &mut editor, &mut cache),
                super::DraftChange::None
            );
            assert_eq!(editor.cursor_byte(), "alpha ".len());

            assert_eq!(
                press(right, &mut app, &mut editor, &mut cache),
                super::DraftChange::None
            );
            assert_eq!(editor.cursor_byte(), "alpha beta".len());

            press(left, &mut app, &mut editor, &mut cache);
            press(delete_right, &mut app, &mut editor, &mut cache);
            assert_eq!(editor.submission_text(), "alpha  ");
        }
    }

    /// macOS keeps the command key for itself, so the readline bindings are
    /// what actually reaches the composer; the super arms only fire where a
    /// terminal is configured to forward that modifier.
    #[test]
    fn line_navigation_answers_readline_and_super_bindings() {
        for (to_start, to_end) in [
            (
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
            ),
            (
                KeyEvent::new(KeyCode::Left, KeyModifiers::SUPER),
                KeyEvent::new(KeyCode::Right, KeyModifiers::SUPER),
            ),
            (
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            ),
        ] {
            let (runtime, _actions) = runtime::interaction_test_runtime(1);
            let mut app = AppState {
                native_composer: NativeComposerState::Clear,
                ..AppState::default()
            };
            let mut editor = Editor::default();
            editor.insert_paste("first line\nsecond line");
            let mut sequence = 1;
            let mut cache = render::HistoryRenderCache::default();
            let mut press = |key, app: &mut AppState, editor: &mut Editor, cache: &mut _| {
                handle_key(key, app, editor, &runtime, &mut sequence, cache).unwrap()
            };

            press(to_start, &mut app, &mut editor, &mut cache);
            assert_eq!(editor.cursor_byte(), "first line\n".len());
            press(to_end, &mut app, &mut editor, &mut cache);
            assert_eq!(editor.cursor_byte(), "first line\nsecond line".len());
        }
    }

    #[test]
    fn super_arrows_reach_the_ends_of_the_draft() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            native_composer: NativeComposerState::Clear,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        editor.insert_paste("first line\nsecond line");
        let mut sequence = 1;
        let mut cache = render::HistoryRenderCache::default();
        let mut press = |key, app: &mut AppState, editor: &mut Editor, cache: &mut _| {
            handle_key(key, app, editor, &runtime, &mut sequence, cache).unwrap()
        };

        press(
            KeyEvent::new(KeyCode::Up, KeyModifiers::SUPER),
            &mut app,
            &mut editor,
            &mut cache,
        );
        assert_eq!(editor.cursor_byte(), 0);
        press(
            KeyEvent::new(KeyCode::Down, KeyModifiers::SUPER),
            &mut app,
            &mut editor,
            &mut cache,
        );
        assert_eq!(editor.cursor_byte(), "first line\nsecond line".len());
    }

    /// macOS sends `^U` for command+backspace, so that is the binding that
    /// actually arrives; the super arms cover terminals that forward the
    /// modifier itself.
    #[test]
    fn line_kills_answer_readline_and_super_bindings() {
        for (kill_left, kill_right) in [
            (
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            ),
            (
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER),
                KeyEvent::new(KeyCode::Delete, KeyModifiers::SUPER),
            ),
        ] {
            let (runtime, _actions) = runtime::interaction_test_runtime(1);
            let mut app = AppState {
                native_composer: NativeComposerState::Clear,
                ..AppState::default()
            };
            let mut editor = Editor::default();
            editor.insert_paste("keep this\ndrop that");
            let mut sequence = 1;
            let mut cache = render::HistoryRenderCache::default();
            let mut press = |key, app: &mut AppState, editor: &mut Editor, cache: &mut _| {
                handle_key(key, app, editor, &runtime, &mut sequence, cache).unwrap()
            };

            assert_eq!(
                press(kill_left, &mut app, &mut editor, &mut cache),
                super::DraftChange::Debounced
            );
            assert_eq!(editor.submission_text(), "keep this\n");

            editor.insert_paste("tail text");
            editor.move_home();
            assert_eq!(
                press(kill_right, &mut app, &mut editor, &mut cache),
                super::DraftChange::Debounced
            );
            assert_eq!(editor.submission_text(), "keep this\n");
        }
    }

    const OCCUPIED_CODEX: &str = concat!(
        "• answer\n",
        "────────\n",
        "› half written prompt\n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    );
    const EMPTY_CODEX: &str = concat!(
        "• answer\n",
        "────────\n",
        "› \n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    );

    /// Opening the overlay over a half-written prompt should not mean switching
    /// back to finish it.
    #[test]
    fn adopting_a_native_draft_moves_the_text_and_clears_the_source() {
        let cleared = std::cell::Cell::new(0);
        let adopted = super::adopt_native_draft(
            AgentKind::Codex,
            || {
                Ok(if cleared.get() == 0 {
                    OCCUPIED_CODEX.to_owned()
                } else {
                    EMPTY_CODEX.to_owned()
                })
            },
            || {
                cleared.set(cleared.get() + 1);
                Ok(())
            },
            8,
            std::time::Duration::ZERO,
        )
        .unwrap()
        .expect("an occupied composer must be adopted");

        assert_eq!(adopted.text, "half written prompt");
        assert!(adopted.cleared);
        assert_eq!(cleared.get(), 1, "clearing must stop once it has worked");
    }

    #[test]
    fn an_empty_native_composer_is_left_alone() {
        let cleared = std::cell::Cell::new(0);
        let adopted = super::adopt_native_draft(
            AgentKind::Codex,
            || Ok(EMPTY_CODEX.to_owned()),
            || {
                cleared.set(cleared.get() + 1);
                Ok(())
            },
            8,
            std::time::Duration::ZERO,
        )
        .unwrap();

        assert_eq!(adopted, None);
        assert_eq!(cleared.get(), 0, "nothing may be sent to an empty composer");
    }

    /// A draft is never lost to a half-finished takeover: the text comes back
    /// even when the source could not be cleared, and the caller says so.
    #[test]
    fn a_draft_that_cannot_be_cleared_is_still_adopted() {
        let adopted = super::adopt_native_draft(
            AgentKind::Codex,
            || Ok(OCCUPIED_CODEX.to_owned()),
            || Ok(()),
            3,
            std::time::Duration::ZERO,
        )
        .unwrap()
        .expect("the text must survive a failed takeover");

        assert_eq!(adopted.text, "half written prompt");
        assert!(!adopted.cleared);
    }

    #[test]
    fn word_hotkeys_stay_blocked_while_the_composer_is_guarded() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            native_composer: NativeComposerState::Occupied,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        editor.insert_paste("alpha beta");
        let before = editor.snapshot();
        let mut sequence = 1;
        let mut cache = render::HistoryRenderCache::default();

        handle_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
            &mut app,
            &mut editor,
            &runtime,
            &mut sequence,
            &mut cache,
        )
        .unwrap();

        assert_eq!(editor.snapshot(), before);
    }

    #[test]
    fn occupied_and_unknown_composers_block_ordinary_editor_mutation_and_submit() {
        for native_composer in [NativeComposerState::Occupied, NativeComposerState::Unknown] {
            let (runtime, actions) = runtime::interaction_test_runtime(1);
            let mut app = AppState {
                native_composer,
                ..AppState::default()
            };
            let mut editor = Editor::default();
            editor.insert_paste("preserved draft");
            let before = editor.snapshot();
            let mut sequence = 1;
            let mut cache = render::HistoryRenderCache::default();

            let change = handle_key(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                &mut app,
                &mut editor,
                &runtime,
                &mut sequence,
                &mut cache,
            )
            .unwrap();

            assert_eq!(change, super::DraftChange::None);
            assert_eq!(editor.snapshot(), before);
            assert!(actions.try_recv().is_err());

            handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &mut app,
                &mut editor,
                &runtime,
                &mut sequence,
                &mut cache,
            )
            .unwrap();
            handle_key(
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                &mut app,
                &mut editor,
                &runtime,
                &mut sequence,
                &mut cache,
            )
            .unwrap();

            assert_eq!(editor.snapshot(), before);
            assert!(app.pending_attachments.is_empty());
            assert!(actions.try_recv().is_err());
        }
    }

    /// The wheel is delivered as arrow keys (alternate scroll), so the arrows
    /// have to drive the history when the composer holds nothing to move a
    /// cursor through — and must not steal the cursor when it does.
    #[test]
    fn arrows_scroll_the_history_only_when_the_composer_has_no_text() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            native_composer: NativeComposerState::Clear,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        let mut sequence = 1;
        let mut cache = render::HistoryRenderCache::default();
        for index in 0..8 {
            app.apply(crate::app::AppEvent::NativeUser(Message::text(
                format!("u{index}"),
                format!("prompt {index}"),
                Some(index),
            )));
        }
        cache.viewport_rows(&app, 40, 3);

        let mut press = |key, app: &mut AppState, editor: &mut Editor, cache: &mut _| {
            handle_key(
                KeyEvent::new(key, KeyModifiers::NONE),
                app,
                editor,
                &runtime,
                &mut sequence,
                cache,
            )
            .unwrap()
        };

        press(KeyCode::Up, &mut app, &mut editor, &mut cache);
        assert!(
            app.scroll_from_bottom > 0,
            "an empty composer must scroll the history"
        );
        let scrolled = app.scroll_from_bottom;
        press(KeyCode::Down, &mut app, &mut editor, &mut cache);
        assert!(app.scroll_from_bottom < scrolled);

        editor.insert_paste("draft line one\ndraft line two");
        let offset = app.scroll_from_bottom;
        let cursor = editor.display_cursor_byte();
        press(KeyCode::Up, &mut app, &mut editor, &mut cache);

        assert_eq!(
            app.scroll_from_bottom, offset,
            "a composer with text must keep the arrows for the cursor"
        );
        assert_ne!(editor.display_cursor_byte(), cursor);
    }

    #[test]
    fn arrows_scroll_the_history_while_the_composer_is_guarded() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            native_composer: NativeComposerState::Occupied,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        editor.insert_paste("preserved draft");
        let before = editor.snapshot();
        let mut sequence = 1;
        let mut cache = render::HistoryRenderCache::default();
        for index in 0..8 {
            app.apply(crate::app::AppEvent::NativeUser(Message::text(
                format!("u{index}"),
                format!("prompt {index}"),
                Some(index),
            )));
        }
        cache.viewport_rows(&app, 40, 3);

        handle_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &mut app,
            &mut editor,
            &runtime,
            &mut sequence,
            &mut cache,
        )
        .unwrap();

        assert!(app.scroll_from_bottom > 0);
        assert_eq!(editor.snapshot(), before);
    }

    #[test]
    fn page_navigation_remains_available_while_composer_is_guarded() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            native_composer: NativeComposerState::Occupied,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        let mut sequence = 1;
        let mut cache = render::HistoryRenderCache::default();
        for index in 0..8 {
            app.apply(crate::app::AppEvent::NativeUser(Message::text(
                format!("u{index}"),
                format!("prompt {index}"),
                Some(index),
            )));
        }
        let _ = render::render_to_string(&app, &editor, 40, 8);
        cache.viewport_rows(&app, 40, 3);

        handle_key(
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
            &mut app,
            &mut editor,
            &runtime,
            &mut sequence,
            &mut cache,
        )
        .unwrap();

        assert!(app.scroll_from_bottom > 0);
    }

    #[test]
    fn guarded_composer_blocks_text_and_staged_image_paste() {
        let (runtime, actions) = runtime::interaction_test_runtime(2);
        let mut app = AppState {
            native_composer: NativeComposerState::Occupied,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        editor.insert_paste("preserved draft");
        let before = editor.snapshot();
        let mut sequence = 1;

        assert_eq!(
            handle_ordinary_paste(
                "additional text",
                &mut app,
                &mut editor,
                &runtime,
                &mut sequence,
            ),
            super::DraftChange::None
        );
        assert_eq!(
            handle_ordinary_paste(
                "/private/tmp/herdr-paste-does-not-exist/image.png",
                &mut app,
                &mut editor,
                &runtime,
                &mut sequence,
            ),
            super::DraftChange::None
        );

        assert_eq!(editor.snapshot(), before);
        assert!(app.pending_attachments.is_empty());
        assert!(actions.try_recv().is_err());
        assert_eq!(sequence, 1);
    }

    #[test]
    fn connection_failure_blocks_input_even_if_composer_snapshot_was_clear() {
        let app = AppState {
            input_enabled: false,
            native_composer: NativeComposerState::Clear,
            ..AppState::default()
        };

        assert!(!super::ordinary_input_allowed(&app));
    }

    #[test]
    fn safe_observation_reenables_preserved_editor_draft() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let original = AgentIdentity {
            pane_id: "w1:p1".into(),
            kind: AgentKind::Codex,
            session_id: "session-1".into(),
            cwd: PathBuf::from("/repo"),
            status: AgentStatus::Done,
        };
        let mut app = AppState {
            native_composer: NativeComposerState::Occupied,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        editor.insert_paste("preserved draft");
        let before = editor.snapshot();
        let mut cache = render::HistoryRenderCache::default();

        apply_runtime_event(
            runtime::RuntimeEvent::Observation(Ok(runtime::SourceObservation {
                identity: original.clone(),
                status_text: "gpt-5.6-sol · /repo · weekly 75% left".into(),
                native_composer: NativeComposerState::Clear,
                blocked_surface: None,
            })),
            &original,
            &mut app,
            &mut editor,
            &mut cache,
            &runtime,
        );

        assert_eq!(app.native_composer, NativeComposerState::Clear);
        assert_eq!(editor.snapshot(), before);
    }

    #[test]
    fn asynchronous_preflight_failure_restores_exact_draft_once() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let original = AgentIdentity {
            pane_id: "w1:p1".into(),
            kind: AgentKind::Codex,
            session_id: "session-1".into(),
            cwd: PathBuf::from("/repo"),
            status: AgentStatus::Done,
        };
        let mut app = AppState::default();
        let mut editor = Editor::default();
        editor.insert_paste(&"private\n".repeat(1_000));
        let submission = editor.take_editor_submission();
        let recovery = submission.recovery.clone();
        app.apply(crate::app::AppEvent::PromptSubmitted {
            local_id: "local-1".into(),
            submission,
            attachments: Vec::new(),
            at_ms: 1,
        });
        let mut cache = render::HistoryRenderCache::default();

        let failed = runtime::RuntimeEvent::Submitted {
            local_id: "local-1".into(),
            result: Err("native composer contains unsent input".into()),
        };
        apply_runtime_event(
            failed,
            &original,
            &mut app,
            &mut editor,
            &mut cache,
            &runtime,
        );

        assert_eq!(app.draft, recovery);
        assert_eq!(editor.snapshot(), recovery);
        assert_eq!(app.turns.len(), 1);
        assert!(matches!(
            app.turns[0].delivery,
            crate::model::Delivery::Failed { .. }
        ));

        apply_runtime_event(
            runtime::RuntimeEvent::Submitted {
                local_id: "local-1".into(),
                result: Err("native composer contains unsent input".into()),
            },
            &original,
            &mut app,
            &mut editor,
            &mut cache,
            &runtime,
        );
        assert_eq!(app.draft, recovery);
        assert_eq!(app.turns.len(), 1);
    }

    #[test]
    fn escape_interrupts_working_agent_even_when_composer_is_unknown() {
        let (runtime, actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            agent_status: AgentStatus::Working,
            native_composer: NativeComposerState::Unknown,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        let mut sequence = 1;
        let mut cache = render::HistoryRenderCache::default();

        handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut app,
            &mut editor,
            &runtime,
            &mut sequence,
            &mut cache,
        )
        .unwrap();

        assert!(matches!(
            actions.try_recv().unwrap(),
            runtime::ActionCommand::Interrupt
        ));
    }

    #[test]
    fn submit_captures_confirmed_attachment_count_before_optimistic_clear() {
        let (runtime, actions) = runtime::interaction_test_runtime(1);
        let mut app = AppState {
            native_composer: NativeComposerState::OwnedAttachments(1),
            draft_attachments: vec![Attachment {
                id: "image-1".into(),
                display: "screen.png".into(),
                native_path: None,
            }],
            ..AppState::default()
        };
        let mut editor = Editor::default();
        editor.insert_paste("describe it");
        let mut sequence = 1;
        let mut cache = render::HistoryRenderCache::default();

        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &mut app,
            &mut editor,
            &runtime,
            &mut sequence,
            &mut cache,
        )
        .unwrap();

        assert!(matches!(
            actions.try_recv().unwrap(),
            runtime::ActionCommand::Submit {
                expected_attachments: 1,
                ..
            }
        ));
        assert!(app.draft_attachments.is_empty());
    }

    #[test]
    fn failed_observation_replaces_a_stale_clear_composer_with_unknown() {
        let (runtime, _actions) = runtime::interaction_test_runtime(1);
        let original = AgentIdentity {
            pane_id: "w1:p1".into(),
            kind: AgentKind::Codex,
            session_id: "session-1".into(),
            cwd: PathBuf::from("/repo"),
            status: AgentStatus::Done,
        };
        let mut app = AppState {
            native_composer: NativeComposerState::Clear,
            ..AppState::default()
        };
        let mut editor = Editor::default();
        let mut cache = render::HistoryRenderCache::default();

        apply_runtime_event(
            runtime::RuntimeEvent::Observation(Err("screen unavailable".into())),
            &original,
            &mut app,
            &mut editor,
            &mut cache,
            &runtime,
        );

        assert_eq!(app.native_composer, NativeComposerState::Unknown);
        assert!(!app.input_enabled);
    }
}
