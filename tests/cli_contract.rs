use herdr_simple_prompts::{AppError, Mode};

#[test]
fn accepts_only_toggle_and_ui_modes() {
    assert_eq!(Mode::parse(Some("toggle")).unwrap(), Mode::Toggle);
    assert_eq!(Mode::parse(Some("ui")).unwrap(), Mode::Ui);
    assert!(Mode::parse(Some("serve")).is_err());
    assert!(Mode::parse(None).is_err());
}

#[test]
fn app_error_keeps_context_in_display_text() {
    let error = AppError::new("startup", "missing HERDR_SOCKET_PATH");
    assert_eq!(
        error.to_string(),
        "startup: missing HERDR_SOCKET_PATH".to_owned()
    );
}
