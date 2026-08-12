use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use herdr_simple_prompts::agent::AgentStatus;
use herdr_simple_prompts::app::AppState;
use herdr_simple_prompts::editor::Editor;
use herdr_simple_prompts::model::{Attachment, Message};
use herdr_simple_prompts::paste::{CompactPromptOverride, PasteRange};
use herdr_simple_prompts::style::StyledText;
use herdr_simple_prompts::ui::interaction::{
    InteractionInput, map_interaction_key, map_interaction_paste,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn native_interaction_keys_use_exact_herdr_names() {
    let cases = [
        (KeyCode::Up, "up"),
        (KeyCode::Down, "down"),
        (KeyCode::Left, "left"),
        (KeyCode::Right, "right"),
        (KeyCode::Tab, "tab"),
        (KeyCode::BackTab, "shift+tab"),
        (KeyCode::Char(' '), "space"),
        (KeyCode::Enter, "enter"),
        (KeyCode::Backspace, "backspace"),
        (KeyCode::Delete, "delete"),
        (KeyCode::Esc, "esc"),
    ];

    for (code, expected) in cases {
        assert_eq!(
            map_interaction_key(key(code)),
            Some(InteractionInput::Key(expected))
        );
    }
}

#[test]
fn printable_interaction_keys_remain_text_and_unsupported_keys_are_ignored() {
    assert_eq!(
        map_interaction_key(key(KeyCode::Char('ж'))),
        Some(InteractionInput::Text("ж".into()))
    );
    assert_eq!(
        map_interaction_key(KeyEvent::new(KeyCode::Char('Ж'), KeyModifiers::SHIFT)),
        Some(InteractionInput::Text("Ж".into()))
    );
    assert_eq!(map_interaction_key(key(KeyCode::PageUp)), None);
    assert_eq!(map_interaction_key(key(KeyCode::PageDown)), None);
    assert_eq!(map_interaction_key(key(KeyCode::F(1))), None);
    assert_eq!(
        map_interaction_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        None
    );
}

#[test]
fn blocked_paste_is_one_full_text_and_never_touches_editor() {
    let content = "first line\n".repeat(1_000);
    let mut editor = Editor::default();
    editor.insert_char('x');
    let before = editor.snapshot();

    assert_eq!(
        map_interaction_paste(&content),
        InteractionInput::Text(content)
    );
    assert_eq!(editor.snapshot(), before);
}

#[test]
fn blocked_surface_and_send_result_do_not_mutate_visible_or_composer_state() {
    let mut app = AppState {
        session_id: "session-1".into(),
        draft_attachments: vec![Attachment {
            id: "image-1".into(),
            display: "diagram.png".into(),
            native_path: Some("/private/tmp/diagram.png".into()),
        }],
        scroll_from_bottom: 13,
        ..AppState::default()
    };
    app.apply(herdr_simple_prompts::app::AppEvent::NativeUser(
        Message::text("prompt-1", "ordinary prompt", Some(1)),
    ));
    app.prompt_displays.push(CompactPromptOverride::new(
        "session-1",
        "prompt-1",
        "large paste",
        vec![PasteRange {
            start_byte: 0,
            end_byte: 11,
            character_count: 1_000,
        }],
    ));
    let _ = app.drain_history_upserts();
    let turns = app.turns.clone();
    let attachments = app.draft_attachments.clone();
    let displays = app.prompt_displays.clone();
    let offset = app.scroll_from_bottom;

    app.update_blocked_surface(
        AgentStatus::Blocked,
        Some(Ok(StyledText {
            text: "Allow this command?".into(),
            runs: Vec::new(),
        })),
    );
    app.apply_interaction_result(Err("native send failed".into()));

    assert_eq!(app.turns, turns);
    assert_eq!(app.draft_attachments, attachments);
    assert_eq!(app.prompt_displays, displays);
    assert_eq!(app.scroll_from_bottom, offset);
    assert_eq!(app.drain_history_upserts(), Vec::new());
    assert_eq!(app.interaction_error.as_deref(), Some("native send failed"));

    app.update_blocked_surface(AgentStatus::Working, None);
    assert!(app.blocked_surface.is_none());
    assert!(app.interaction_error.is_none());
    assert_eq!(app.turns, turns);
    assert_eq!(app.draft_attachments, attachments);
    assert_eq!(app.prompt_displays, displays);
    assert_eq!(app.scroll_from_bottom, offset);
    app.apply_interaction_result(Err("late native send failure".into()));
    assert!(app.interaction_error.is_none());
}
