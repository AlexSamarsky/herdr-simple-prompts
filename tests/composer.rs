use herdr_simple_prompts::agent::AgentKind;
use herdr_simple_prompts::composer::{
    ComposerAccess, NativeComposerState, classify_native_composer,
};
use herdr_simple_prompts::style::{AnsiColor, StyleModifiers, StyleRun, StyledText};

fn styled_range(text: &str, needle: &str, foreground: AnsiColor, dim: bool) -> StyledText {
    let start_byte = text.find(needle).expect("fixture contains styled text");
    StyledText {
        text: text.to_owned(),
        runs: vec![StyleRun {
            start_byte,
            end_byte: start_byte + needle.len(),
            foreground: Some(foreground),
            background: None,
            modifiers: StyleModifiers {
                dim,
                ..StyleModifiers::default()
            },
        }],
    }
}

fn plain(text: &str) -> StyledText {
    StyledText {
        text: text.to_owned(),
        runs: Vec::new(),
    }
}

fn codex_surface(prompt: &str) -> String {
    format!("────────\n• answer\n────────\n› {prompt}\ngpt-5.6-sol xhigh · /repo · weekly 75% left")
}

fn claude_surface(prompt: &str) -> String {
    format!("⏺ answer\n────────────────\n❯ {prompt}\n────────────────\nClaude Opus · /repo")
}

#[test]
fn codex_dim_only_placeholder_is_clear() {
    let text = codex_surface("Write a prompt");
    let start_byte = text.find("Write a prompt").unwrap();
    let surface = StyledText {
        text,
        runs: vec![StyleRun {
            start_byte,
            end_byte: start_byte + "Write a prompt".len(),
            foreground: None,
            background: None,
            modifiers: StyleModifiers {
                dim: true,
                ..StyleModifiers::default()
            },
        }],
    };

    assert_eq!(
        classify_native_composer(AgentKind::Codex, &surface),
        NativeComposerState::Clear
    );
}

#[test]
fn codex_dim_suggestion_is_clear_without_matching_literal_copy() {
    let text = codex_surface("Summarize recent commits");
    let surface = styled_range(
        &text,
        "Summarize recent commits",
        AnsiColor::Indexed(8),
        false,
    );

    assert_eq!(
        classify_native_composer(AgentKind::Codex, &surface),
        NativeComposerState::Clear
    );
}

#[test]
fn codex_plain_text_is_occupied() {
    assert_eq!(
        classify_native_composer(AgentKind::Codex, &plain(&codex_surface("unsent text"))),
        NativeComposerState::Occupied
    );
}

#[test]
fn codex_exact_image_tokens_are_counted() {
    assert_eq!(
        classify_native_composer(
            AgentKind::Codex,
            &plain(&codex_surface("[Image #1]  [Image #2]")),
        ),
        NativeComposerState::OwnedAttachments(2)
    );
}

#[test]
fn codex_image_token_mixed_with_text_is_occupied() {
    assert_eq!(
        classify_native_composer(
            AgentKind::Codex,
            &plain(&codex_surface("[Image #1] explain this")),
        ),
        NativeComposerState::Occupied
    );
}

#[test]
fn codex_missing_footer_and_truncated_surface_are_unknown() {
    assert_eq!(
        classify_native_composer(AgentKind::Codex, &plain("────────\n• answer\n────────\n› "),),
        NativeComposerState::Unknown
    );
    assert_eq!(
        classify_native_composer(AgentKind::Codex, &plain("• answer\n────────")),
        NativeComposerState::Unknown
    );
}

#[test]
fn historical_codex_prompt_followed_by_a_new_block_is_not_current() {
    let surface = plain(concat!(
        "────────\n",
        "› old text\n",
        "• Ran command\n",
        "────────\n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    ));

    assert_eq!(
        classify_native_composer(AgentKind::Codex, &surface),
        NativeComposerState::Unknown
    );
}

#[test]
fn claude_prompt_box_classifies_clear_text_and_images() {
    let clear_text = claude_surface("Write a prompt");
    let clear = styled_range(&clear_text, "Write a prompt", AnsiColor::BrightBlack, false);
    assert_eq!(
        classify_native_composer(AgentKind::Claude, &clear),
        NativeComposerState::Clear
    );
    assert_eq!(
        classify_native_composer(AgentKind::Claude, &plain(&claude_surface("unsent text")),),
        NativeComposerState::Occupied
    );
    assert_eq!(
        classify_native_composer(
            AgentKind::Claude,
            &plain(&claude_surface("[Image #7]\n  [Image #9]")),
        ),
        NativeComposerState::OwnedAttachments(2)
    );
}

#[test]
fn claude_historical_prompt_box_followed_by_output_is_unknown() {
    let surface = plain(concat!(
        "────────────────\n",
        "❯ [Image #1]\n",
        "────────────────\n",
        "⏺ later answer\n",
        "Claude Opus · /repo",
    ));

    assert_eq!(
        classify_native_composer(AgentKind::Claude, &surface),
        NativeComposerState::Unknown
    );
}

#[test]
fn claude_requires_both_prompt_box_rules() {
    assert_eq!(
        classify_native_composer(
            AgentKind::Claude,
            &plain("⏺ answer\n────────────────\n❯ \nClaude Opus · /repo"),
        ),
        NativeComposerState::Unknown
    );
}

#[test]
fn arbitrary_rgb_text_is_not_treated_as_a_placeholder() {
    let text = codex_surface("Summarize recent commits");
    let surface = styled_range(
        &text,
        "Summarize recent commits",
        AnsiColor::Rgb(65, 66, 67),
        true,
    );

    assert_eq!(
        classify_native_composer(AgentKind::Codex, &surface),
        NativeComposerState::Occupied
    );
}

#[test]
fn codex_decorated_boundary_requires_a_numeric_elapsed_label() {
    let malformed = plain(concat!(
        "• answer\n",
        "─ Worked for eventually ────────\n",
        "› \n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    ));
    assert_eq!(
        classify_native_composer(AgentKind::Codex, &malformed),
        NativeComposerState::Unknown
    );

    let valid = plain(concat!(
        "• answer\n",
        "─ Worked for 2m 3s ────────\n",
        "› \n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    ));
    assert_eq!(
        classify_native_composer(AgentKind::Codex, &valid),
        NativeComposerState::Clear
    );
}

#[test]
fn access_policy_requires_an_exact_attachment_count() {
    assert_eq!(NativeComposerState::Clear.access(0), ComposerAccess::Ready);
    assert_eq!(
        NativeComposerState::Clear.access(1),
        ComposerAccess::Occupied
    );
    assert_eq!(
        NativeComposerState::OwnedAttachments(2).access(2),
        ComposerAccess::Ready
    );
    assert_eq!(
        NativeComposerState::OwnedAttachments(2).access(1),
        ComposerAccess::Occupied
    );
    assert_eq!(
        NativeComposerState::Occupied.access(0),
        ComposerAccess::Occupied
    );
    assert_eq!(
        NativeComposerState::Unknown.access(0),
        ComposerAccess::Unknown
    );
}
