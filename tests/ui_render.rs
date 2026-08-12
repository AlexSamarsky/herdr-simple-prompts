use herdr_simple_prompts::agent::AgentStatus;
use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::editor::Editor;
use herdr_simple_prompts::model::Attachment;
use herdr_simple_prompts::model::Message;
use herdr_simple_prompts::ui::render::render_to_string;
use std::time::{Duration, Instant};

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
