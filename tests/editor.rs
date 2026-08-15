use herdr_simple_prompts::editor::{
    Editor, EditorChunk, EditorCommand, EditorSnapshot, Key, map_key, staged_image_path,
};
use herdr_simple_prompts::model::Attachment;
use herdr_simple_prompts::paste::{LARGE_PASTE_CHARS, PasteRange, large_paste_marker};

#[test]
fn editor_chunks_round_trip_through_the_tagged_serde_contract() {
    let snapshot = EditorSnapshot {
        chunks: vec![
            EditorChunk::Text("before".to_owned()),
            EditorChunk::LargePaste {
                source_text: "界".repeat(LARGE_PASTE_CHARS),
                character_count: LARGE_PASTE_CHARS,
            },
        ],
    };

    let serialized = serde_json::to_value(&snapshot).unwrap();

    assert_eq!(
        serialized,
        serde_json::json!({
            "chunks": [
                {"kind": "text", "value": "before"},
                {
                    "kind": "large_paste",
                    "value": {
                        "source_text": "界".repeat(LARGE_PASTE_CHARS),
                        "character_count": LARGE_PASTE_CHARS
                    }
                }
            ]
        })
    );
    assert_eq!(
        serde_json::from_value::<EditorSnapshot>(serialized).unwrap(),
        snapshot
    );
}

#[test]
fn below_threshold_paste_remains_plain_text() {
    let mut editor = Editor::default();
    let pasted = "界".repeat(LARGE_PASTE_CHARS - 1);

    editor.insert_paste(&pasted);

    assert_eq!(editor.display_text(), pasted);
    assert_eq!(editor.submission_text(), pasted);
    assert_eq!(
        editor.snapshot(),
        EditorSnapshot::plain(pasted),
        "a small paste should remain ordinary text in recovery data"
    );
}

#[test]
fn very_large_paste_is_compact_for_display_and_lossless_for_submission() {
    let mut editor = Editor::default();
    let pasted = format!("first\n{}\nlast", "я".repeat(1_000_000));
    let character_count = pasted.chars().count();
    let marker = large_paste_marker(character_count);

    editor.insert_paste(&pasted);

    assert_eq!(editor.display_text(), marker);
    assert!(!editor.display_text().contains(&"я".repeat(1_000_000)));
    assert_eq!(editor.text(), pasted);
    assert_eq!(editor.submission_text(), pasted);
    assert_eq!(
        editor.snapshot().chunks,
        vec![EditorChunk::LargePaste {
            source_text: pasted.clone(),
            character_count,
        }]
    );

    let submission = editor.take_editor_submission();
    let expected_recovery = EditorSnapshot {
        chunks: vec![EditorChunk::LargePaste {
            source_text: pasted.clone(),
            character_count,
        }],
    };

    assert_eq!(submission.complete_text, pasted);
    assert_eq!(submission.display_text, marker);
    assert_eq!(submission.recovery, expected_recovery);
    assert_eq!(
        submission.paste_ranges,
        vec![PasteRange {
            start_byte: 0,
            end_byte: submission.complete_text.len(),
            character_count,
        }]
    );
    assert_eq!(editor.text(), "");
    assert_eq!(editor.display_text(), "");
    assert_eq!(editor.cursor_byte(), 0);
    assert_eq!(editor.display_cursor_byte(), 0);
    assert!(editor.is_empty());

    let mut legacy_editor = Editor::default();
    legacy_editor.insert_paste(&pasted);
    assert_eq!(legacy_editor.take_submission(), pasted);
    assert!(legacy_editor.is_empty());
}

#[test]
fn multiple_large_pastes_remain_ordered_separate_and_atomic() {
    let mut editor = Editor::default();
    let first = "a".repeat(1_000);
    let second = "界".repeat(1_001);
    editor.insert_char('>');
    editor.insert_paste(&first);
    editor.insert_char('|');
    editor.insert_paste(&second);
    editor.insert_char('<');

    let expected_display = format!(
        ">{}|{}<",
        large_paste_marker(1_000),
        large_paste_marker(1_001)
    );
    let expected_submission = format!(">{first}|{second}<");
    assert_eq!(editor.display_text(), expected_display);
    assert_eq!(editor.submission_text(), expected_submission);

    let expected_ranges = vec![
        PasteRange {
            start_byte: 1,
            end_byte: 1 + first.len(),
            character_count: 1_000,
        },
        PasteRange {
            start_byte: 1 + first.len() + 1,
            end_byte: 1 + first.len() + 1 + second.len(),
            character_count: 1_001,
        },
    ];
    let snapshot = EditorSnapshot {
        chunks: vec![
            EditorChunk::Text(">".to_owned()),
            EditorChunk::LargePaste {
                source_text: first.clone(),
                character_count: 1_000,
            },
            EditorChunk::Text("|".to_owned()),
            EditorChunk::LargePaste {
                source_text: second,
                character_count: 1_001,
            },
            EditorChunk::Text("<".to_owned()),
        ],
    };
    assert_eq!(editor.snapshot(), snapshot);
    assert_eq!(
        editor.take_editor_submission().paste_ranges,
        expected_ranges
    );

    let mut restored = Editor::default();
    restored.insert_char('x');
    restored.replace_snapshot(snapshot.clone());

    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(restored.display_text(), expected_display);
    assert_eq!(restored.submission_text(), expected_submission);

    restored.move_left();
    restored.backspace();

    assert_eq!(restored.submission_text(), format!(">{first}|<"));
    assert_eq!(
        restored.display_text(),
        format!(">{}|<", large_paste_marker(1_000))
    );
}

#[test]
fn cursor_crosses_a_large_paste_on_display_atom_boundaries() {
    let mut editor = Editor::default();
    let pasted = "界".repeat(LARGE_PASTE_CHARS);
    editor.insert_paste(&pasted);
    let source_after = editor.cursor_byte();
    let display_after = editor.display_cursor_byte();

    editor.move_left();

    assert_eq!(editor.cursor_byte(), 0);
    assert_eq!(source_after - editor.cursor_byte(), pasted.len());
    assert_eq!(editor.display_cursor_byte(), 0);
    assert_eq!(
        display_after - editor.display_cursor_byte(),
        large_paste_marker(LARGE_PASTE_CHARS).len()
    );

    editor.move_right();
    editor.delete();
    editor.move_left();
    editor.delete();
    assert_eq!(editor.submission_text(), "");
}

#[test]
fn normal_editing_keeps_source_and_display_in_sync() {
    let mut editor = Editor::default();
    editor.insert_paste("abc");
    editor.move_left();
    editor.insert_char('X');
    editor.delete();
    editor.backspace();

    assert_eq!(editor.text(), "ab");
    assert_eq!(editor.display_text(), "ab");
    assert_eq!(editor.cursor_byte(), 2);
    assert_eq!(editor.display_cursor_byte(), 2);
}

#[test]
fn shift_enter_and_ctrl_j_insert_newline_while_enter_submits() {
    assert_eq!(map_key(Key::Enter), EditorCommand::Submit);
    assert_eq!(map_key(Key::ShiftEnter), EditorCommand::Newline);
    assert_eq!(map_key(Key::Ctrl('j')), EditorCommand::Newline);
    assert_eq!(map_key(Key::Character('x')), EditorCommand::Insert('x'));
    assert_eq!(map_key(Key::Ctrl('x')), EditorCommand::None);
}

#[test]
fn editing_never_splits_a_unicode_scalar() {
    let mut editor = Editor::default();
    editor.insert_paste("aя🙂z");
    editor.move_left();
    editor.backspace();

    assert_eq!(editor.text(), "aяz");
    assert!(editor.text().is_char_boundary(editor.cursor_byte()));
    assert!(
        editor
            .display_text()
            .is_char_boundary(editor.display_cursor_byte())
    );
}

#[test]
fn home_and_end_move_within_the_display_line() {
    let mut editor = Editor::default();
    editor.replace("alpha\nbeta\ngamma");

    editor.move_home();
    editor.insert_char('>');
    editor.move_end();
    editor.insert_char('<');

    assert_eq!(editor.text(), "alpha\nbeta\n>gamma<");
}

#[test]
fn vertical_movement_preserves_the_preferred_display_column() {
    let mut editor = Editor::default();
    editor.replace("abcd\nxy\n1234");

    editor.move_up();
    assert_eq!(editor.cursor_byte(), 7);
    editor.move_up();
    assert_eq!(editor.cursor_byte(), 4);
    editor.move_down();
    assert_eq!(editor.cursor_byte(), 7);
}

#[test]
fn legacy_take_submission_is_lossless_and_clears_the_editor() {
    let mut editor = Editor::default();
    let source = format!("before{}after", "🙂".repeat(LARGE_PASTE_CHARS));
    editor.insert_paste(&source);

    assert_eq!(editor.take_submission(), source);
    assert_eq!(editor.text(), "");
    assert_eq!(editor.display_text(), "");
    assert_eq!(editor.snapshot(), EditorSnapshot::default());
    assert!(editor.is_empty());
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

#[test]
fn word_motions_step_over_whole_words() {
    let mut editor = Editor::default();
    editor.insert_paste("hello world  foo");

    for expected in ["hello world  ".len(), "hello ".len(), 0, 0] {
        editor.move_word_left();
        assert_eq!(editor.cursor_byte(), expected);
    }
    for expected in [
        "hello".len(),
        "hello world".len(),
        "hello world  foo".len(),
        "hello world  foo".len(),
    ] {
        editor.move_word_right();
        assert_eq!(editor.cursor_byte(), expected);
    }
}

#[test]
fn word_deletion_removes_one_word_at_a_time() {
    let mut editor = Editor::default();
    editor.insert_paste("alpha beta gamma");

    editor.delete_word_left();
    assert_eq!(editor.submission_text(), "alpha beta ");
    editor.delete_word_left();
    assert_eq!(editor.submission_text(), "alpha ");

    editor.move_home();
    editor.delete_word_right();
    assert_eq!(editor.submission_text(), " ");
    editor.delete_word_right();
    assert_eq!(editor.submission_text(), "");
    editor.delete_word_left();
    editor.delete_word_right();
    assert_eq!(editor.submission_text(), "");
}

/// A collapsed paste stands for a block of text the composer is not showing, so
/// a word operation has to treat it as one unit instead of stepping inside it.
#[test]
fn a_collapsed_paste_counts_as_a_single_word() {
    let body = "x".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste("before ");
    editor.insert_paste(&body);
    editor.insert_paste(" after");

    editor.delete_word_left();
    assert_eq!(editor.submission_text(), format!("before {body} "));

    editor.delete_word_left();
    assert_eq!(
        editor.submission_text(),
        "before ",
        "the paste must be removed whole, never split"
    );
}

#[test]
fn word_motions_stop_at_a_collapsed_paste_edge() {
    let body = "x".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste("before ");
    editor.insert_paste(&body);
    editor.insert_paste(" after");

    editor.move_word_left();
    assert_eq!(editor.cursor_byte(), format!("before {body} ").len());
    editor.move_word_left();
    assert_eq!(editor.cursor_byte(), "before ".len());
    editor.move_word_left();
    assert_eq!(editor.cursor_byte(), 0);
}

#[test]
fn line_kills_cut_only_the_current_line() {
    let mut editor = Editor::default();
    editor.insert_paste("first line\nsecond line\nthird line");
    editor.move_up();
    editor.move_end();

    editor.delete_to_line_start();
    assert_eq!(editor.submission_text(), "first line\n\nthird line");

    editor.move_home();
    editor.delete_to_line_end();
    assert_eq!(editor.submission_text(), "first line\n\nthird line");

    editor.move_document_start();
    editor.delete_to_line_end();
    assert_eq!(editor.submission_text(), "\n\nthird line");
    editor.delete_to_line_start();
    assert_eq!(editor.submission_text(), "\n\nthird line");
}

/// A collapsed paste is one atom, so a line kill removes it whole or not at
/// all — it can never leave half of a body the composer is not showing.
#[test]
fn line_kills_keep_a_collapsed_paste_whole() {
    let body = "x".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste("head ");
    editor.insert_paste(&body);
    editor.insert_paste(" tail");

    editor.delete_to_line_start();
    assert_eq!(editor.submission_text(), "");
}

/// An image holds a place in the line rather than a shelf above it: it shows
/// where it was put, moves with the text around it, and contributes nothing to
/// the prompt — the image itself lives in the native composer.
#[test]
fn an_attachment_holds_its_place_in_the_line() {
    let mut editor = Editor::default();
    editor.insert_paste("describe ");
    editor.insert_attachment(Attachment {
        id: "image-1".into(),
        display: "screen.png".into(),
        native_path: None,
    });
    editor.insert_paste("please");

    assert_eq!(editor.display_text(), "describe [Image #1] please");
    assert_eq!(editor.submission_text(), "describe please");
    assert_eq!(editor.attachments().len(), 1);
    assert_eq!(editor.attachments()[0].id, "image-1");
}

#[test]
fn attachments_are_numbered_by_the_order_they_sit_in() {
    let mut editor = Editor::default();
    for id in ["second", "third"] {
        editor.insert_attachment(Attachment {
            id: id.into(),
            display: id.into(),
            native_path: None,
        });
    }
    editor.move_document_start();
    editor.insert_attachment(Attachment {
        id: "first".into(),
        display: "first".into(),
        native_path: None,
    });

    assert_eq!(editor.display_text(), "[Image #1] [Image #2] [Image #3] ");
    assert_eq!(
        editor
            .attachments()
            .iter()
            .map(|attachment| attachment.id.clone())
            .collect::<Vec<_>>(),
        ["first", "second", "third"],
    );
}

#[test]
fn an_attachment_counts_as_one_word_and_survives_a_snapshot() {
    let mut editor = Editor::default();
    editor.insert_attachment(Attachment {
        id: "image-1".into(),
        display: "screen.png".into(),
        native_path: None,
    });
    editor.insert_paste("describe it");

    editor.move_word_left();
    assert_eq!(editor.cursor_byte(), "describe ".len());
    editor.move_word_left();
    assert_eq!(editor.cursor_byte(), 0, "the marker is a single word");

    let snapshot = editor.snapshot();
    let mut restored = Editor::default();
    restored.replace_snapshot(snapshot);
    assert_eq!(restored.display_text(), "[Image #1] describe it");
    assert_eq!(restored.submission_text(), "describe it");
}

/// A marker is a claim about the native composer, and the composer can move on
/// without us. A draft that outlived its image used to insist the pane still
/// held one, which guarded the overlay's own input for as long as it survived.
#[test]
fn markers_can_be_brought_back_in_line_with_the_pane() {
    let mut editor = Editor::default();
    for id in ["first", "second"] {
        editor.insert_attachment(Attachment {
            id: id.into(),
            display: id.into(),
            native_path: None,
        });
    }
    editor.insert_paste("describe it");

    editor.retain_attachments(1);
    assert_eq!(editor.display_text(), "[Image #1] describe it");
    assert_eq!(editor.attachments().len(), 1);
    assert_eq!(editor.attachments()[0].id, "first");
    assert_eq!(editor.submission_text(), "describe it");

    editor.retain_attachments(0);
    assert_eq!(editor.display_text(), "describe it");
    assert!(editor.attachments().is_empty());
    assert_eq!(editor.submission_text(), "describe it");

    editor.retain_attachments(5);
    assert_eq!(editor.display_text(), "describe it", "nothing is invented");
}

/// The marker stands for a picture the agent is holding, and removing it here
/// has to remove it there — which is not wired up. Until it is, every deletion
/// stops at the marker, so the two sides never disagree about how many images
/// exist and a prompt is never refused over a phantom one.
#[test]
fn deletions_stop_at_an_image_instead_of_passing_through_it() {
    let attachment = || Attachment {
        id: "image-1".into(),
        display: "Image #7".into(),
        native_path: None,
    };

    let mut editor = Editor::default();
    editor.insert_attachment(attachment());
    editor.insert_paste("tail");
    editor.move_home();
    editor.move_right();
    for _ in 0..4 {
        editor.backspace();
    }
    assert_eq!(
        editor.attachments().len(),
        1,
        "backspace stops at the image"
    );

    editor.delete_word_left();
    editor.delete_to_line_start();
    assert_eq!(
        editor.attachments().len(),
        1,
        "word and line kills stop too"
    );

    editor.move_document_start();
    editor.delete();
    editor.delete_word_right();
    editor.delete_to_line_end();
    assert_eq!(editor.attachments().len(), 1, "and so do forward deletions");
    assert_eq!(
        editor.submission_text(),
        "tail",
        "the text around it is intact"
    );
}

/// Agents number an image when it is pasted and keep that number for the rest of
/// the session, so the overlay shows the label the pane gave it rather than the
/// place it happens to sit in.
#[test]
fn an_image_keeps_the_number_the_pane_gave_it() {
    let mut editor = Editor::default();
    for label in ["Image #3", "Image #4"] {
        editor.insert_attachment(Attachment {
            id: label.into(),
            display: label.into(),
            native_path: None,
        });
    }

    assert_eq!(editor.display_text(), "[Image #3] [Image #4] ");
}
