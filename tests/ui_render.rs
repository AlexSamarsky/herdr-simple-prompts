use herdr_simple_prompts::agent::AgentStatus;
use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::editor::Editor;
use herdr_simple_prompts::model::Attachment;
use herdr_simple_prompts::model::Message;
use herdr_simple_prompts::ui::render::{render_to_buffer, render_to_string};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::time::{Duration, Instant};

fn rendered_buffer(app: &AppState, width: u16, height: u16) -> Buffer {
    render_to_buffer(app, &Editor::default(), width, height)
}

#[test]
fn prompt_band_and_answer_label_distinguish_roles_without_color_only() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "check dns",
        Some(1),
    )));
    app.apply(AppEvent::NativeFinal(Message::text(
        "a1",
        "zone is pending",
        Some(2),
    )));

    let rendered = render_to_string(&app, &Editor::default(), 50, 14);
    let buffer = rendered_buffer(&app, 50, 14);

    assert!(rendered.contains("YOU  check dns"));
    assert!(rendered.contains("ANSWER"));
    let prompt_row = (0..14)
        .find(|&row| buffer[(0, row)].symbol() == "Y")
        .expect("prompt row should start with YOU");
    let prompt_style = buffer[(0, prompt_row)].style();
    assert_eq!(prompt_style.bg, Some(Color::DarkGray));
    assert!(prompt_style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn working_prompt_is_above_composer_and_footer() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1",
        "run tests",
        Some(1),
    )));
    app.agent_status = AgentStatus::Working;
    app.working_since = Some(Instant::now() - Duration::from_secs(2));
    let editor = Editor::default();

    let rendered = render_to_string(&app, &editor, 80, 24);

    let prompt = rendered.find("run tests").unwrap();
    let working = rendered.find("Working (2s · esc to interrupt)").unwrap();
    let composer = rendered.find("Write a prompt").unwrap();
    assert!(prompt < working && working < composer);
}

#[test]
fn composer_shows_attached_images_before_submission() {
    let mut app = AppState::default();
    app.draft_attachments.push(Attachment {
        id: "image-1".into(),
        display: "screen.png".into(),
        native_path: None,
    });

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("[Image #1] screen.png"));
}

#[test]
fn composer_marks_images_as_pending_until_native_verification() {
    let mut app = AppState::default();
    app.pending_attachments.push(Attachment {
        id: "pending-1".into(),
        display: "screen.png".into(),
        native_path: Some("/private/tmp/screen.png".into()),
    });

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("screen.png (verifying…)"));
}

#[test]
fn only_normalized_messages_reach_the_view() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "hello", Some(1))));
    app.apply(AppEvent::NativeFinal(Message::text("a1", "done", Some(2))));

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("hello"));
    assert!(rendered.contains("done"));
    assert!(!rendered.contains("tool_call"));
    assert!(!rendered.contains("reasoning"));
}

#[test]
fn history_starts_at_the_bottom_and_page_up_moves_toward_older_turns() {
    let mut app = AppState::default();
    for index in 0..20 {
        app.apply(AppEvent::NativeUser(Message::text(
            format!("u{index}"),
            format!("prompt {index}"),
            Some(index),
        )));
        app.apply(AppEvent::NativeFinal(Message::text(
            format!("a{index}"),
            format!("answer {index}"),
            Some(index),
        )));
    }

    let newest = render_to_string(&app, &Editor::default(), 50, 12);
    assert!(newest.contains("prompt 19"));
    assert!(!newest.contains("prompt 0"));

    app.scroll_from_bottom = u16::MAX;
    let oldest = render_to_string(&app, &Editor::default(), 50, 12);
    assert!(oldest.contains("prompt 0"));
    assert!(!oldest.contains("prompt 19"));
}

#[test]
fn disabled_composer_explains_that_the_source_must_be_reopened() {
    let app = AppState {
        input_enabled: false,
        connection_error: Some("source agent session changed".into()),
        ..AppState::default()
    };

    let rendered = render_to_string(&app, &Editor::default(), 80, 24);

    assert!(rendered.contains("Input disabled"));
    assert!(rendered.contains("source agent session changed"));
}

#[test]
fn composer_shows_large_paste_marker_instead_of_log_body() {
    let app = AppState::default();
    let mut editor = Editor::default();
    editor.insert_char('>');
    editor.insert_paste(&"private-log-line\n".repeat(1_000));
    editor.insert_char('<');

    let rendered = render_to_string(&app, &editor, 80, 24);

    assert!(rendered.contains("Pasted Content"));
    assert!(rendered.contains("chars"));
    assert!(rendered.contains('>'));
    assert!(rendered.contains('<'));
    assert!(!rendered.contains("private-log-line"));
}
