use herdr_simple_prompts::editor::{Editor, EditorCommand, Key, map_key, staged_image_path};

#[test]
fn paste_preserves_two_megabytes_and_newlines() {
    let mut editor = Editor::default();
    let pasted = format!("first\n{}\nlast", "я".repeat(1_000_000));

    editor.insert_paste(&pasted);

    assert_eq!(editor.text(), pasted);
    assert!(editor.text().is_char_boundary(editor.cursor_byte()));
}

#[test]
fn shift_enter_and_ctrl_j_insert_newline_while_enter_submits() {
    assert_eq!(map_key(Key::Enter), EditorCommand::Submit);
    assert_eq!(map_key(Key::ShiftEnter), EditorCommand::Newline);
    assert_eq!(map_key(Key::Ctrl('j')), EditorCommand::Newline);
}

#[test]
fn editing_never_splits_a_unicode_scalar() {
    let mut editor = Editor::default();
    editor.insert_paste("aя🙂z");
    editor.move_left();
    editor.backspace();

    assert_eq!(editor.text(), "aяz");
    assert!(editor.text().is_char_boundary(editor.cursor_byte()));
}

#[test]
fn only_existing_herdr_staged_image_paths_are_recognized() {
    let directory = std::env::temp_dir().join(format!(
        "herdr-clipboard-images-{}-editor-test",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let image = directory.join("Image One.PNG");
    std::fs::write(&image, b"not-real-image-but-existing").unwrap();

    assert_eq!(staged_image_path(image.to_str().unwrap()).unwrap(), image);
    assert!(staged_image_path("normal multiline\ntext").is_none());
    assert!(staged_image_path(directory.join("missing.png").to_str().unwrap()).is_none());

    std::fs::remove_dir_all(directory).unwrap();
}
