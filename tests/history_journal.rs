use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::editor::Editor;
use herdr_simple_prompts::history::{
    HistoryJournal, HistoryWriter, PersistedPresentation, VisibleAttachment, VisibleHistoryRecord,
    VisibleRole,
};
use herdr_simple_prompts::model::{Attachment, Message};
use herdr_simple_prompts::paste::{LARGE_PASTE_CHARS, fingerprint};
use herdr_simple_prompts::style::{AnsiColor, MessagePresentation, StyleModifiers, StyleRun};
use std::os::unix::fs::PermissionsExt;

fn test_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "herdr-simple-prompts-history-{label}-{}",
        std::process::id()
    ))
}

fn prompt(id: &str, turn_id: &str, order: u64, text: &str) -> VisibleHistoryRecord {
    VisibleHistoryRecord {
        version: 1,
        role: VisibleRole::Prompt,
        stable_id: id.into(),
        turn_id: turn_id.into(),
        order,
        text: text.into(),
        attachments: Vec::new(),
        timestamp_ms: Some(order),
        text_fingerprint: fingerprint(text),
        presentation: PersistedPresentation::Plain,
    }
}

fn final_record(id: &str, turn_id: &str, order: u64, text: &str) -> VisibleHistoryRecord {
    VisibleHistoryRecord {
        version: 1,
        role: VisibleRole::Final,
        stable_id: id.into(),
        turn_id: turn_id.into(),
        order,
        text: text.into(),
        attachments: Vec::new(),
        timestamp_ms: Some(order),
        text_fingerprint: fingerprint(text),
        presentation: PersistedPresentation::Fallback,
    }
}

fn native_run(text: &str) -> StyleRun {
    StyleRun {
        start_byte: 0,
        end_byte: text.len(),
        foreground: Some(AnsiColor::Green),
        background: None,
        modifiers: StyleModifiers {
            bold: true,
            ..StyleModifiers::default()
        },
    }
}

fn append_json_line(path: &std::path::Path, value: &impl serde::Serialize) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    serde_json::to_writer(&mut file, value).unwrap();
    file.write_all(b"\n").unwrap();
}

#[test]
fn journal_path_is_scoped_and_sanitized_beneath_the_state_root() {
    let root = test_root("path");
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    assert_eq!(journal.path(), root.join("history/w1_p1/session-1.jsonl"));

    let hostile = HistoryJournal::at(&root, "../w1/p1", "../session/1").unwrap();
    assert!(hostile.path().starts_with(root.join("history")));
    assert!(!hostile.path().to_string_lossy().contains("/../"));
    assert!(HistoryJournal::at(&root, "w1:p1", "").is_err());
}

#[test]
fn append_creates_private_directories_and_file() {
    let root = test_root("modes");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    journal.append(&prompt("u1", "u1", 1, "question")).unwrap();

    for directory in [&root, &root.join("history"), &root.join("history/w1_p1")] {
        assert_eq!(
            std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert_eq!(
        std::fs::metadata(journal.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn latest_valid_record_per_role_and_stable_id_wins_in_order() {
    let root = test_root("latest");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    journal.append(&prompt("u1", "u1", 1, "old")).unwrap();
    journal.append(&prompt("u2", "u2", 2, "second")).unwrap();
    journal.append(&prompt("u1", "u1", 1, "new")).unwrap();

    let loaded = journal.load().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].stable_id, "u1");
    assert_eq!(loaded[0].text, "new");
    assert_eq!(loaded[1].stable_id, "u2");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn later_native_style_upsert_replaces_fallback_without_changing_order() {
    let root = test_root("style-upsert");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let fallback = final_record("a1", "u1", 2, "answer");
    let mut native = fallback.clone();
    native.presentation = PersistedPresentation::NativeAnsi(vec![native_run("answer")]);
    journal.append(&fallback).unwrap();
    journal.append(&native).unwrap();

    assert_eq!(journal.load().unwrap(), vec![native]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn incomplete_final_line_is_ignored_until_terminated() {
    let root = test_root("incomplete");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let first = prompt("u1", "u1", 1, "safe");
    journal.append(&first).unwrap();
    let mut bytes = serde_json::to_vec(&prompt("u2", "u2", 2, "partial")).unwrap();
    bytes.truncate(bytes.len() / 2);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap()
        .write_all(&bytes)
        .unwrap();

    assert_eq!(journal.load().unwrap(), vec![first]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_upserts_do_not_poison_the_last_valid_record() {
    let root = test_root("invalid");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let valid = final_record("a1", "u1", 2, "a界b");
    journal.append(&valid).unwrap();

    let mut invalid_version = valid.clone();
    invalid_version.version = 2;
    append_json_line(journal.path(), &invalid_version);
    let mut invalid_fingerprint = valid.clone();
    invalid_fingerprint.text_fingerprint ^= 1;
    append_json_line(journal.path(), &invalid_fingerprint);
    let mut split_scalar = valid.clone();
    split_scalar.presentation = PersistedPresentation::NativeAnsi(vec![StyleRun {
        start_byte: 1,
        end_byte: 2,
        ..native_run("a界b")
    }]);
    append_json_line(journal.path(), &split_scalar);
    let mut overlap = valid.clone();
    overlap.presentation = PersistedPresentation::NativeAnsi(vec![
        StyleRun {
            start_byte: 0,
            end_byte: 4,
            ..native_run("a界b")
        },
        StyleRun {
            start_byte: 1,
            end_byte: 5,
            ..native_run("a界b")
        },
    ]);
    append_json_line(journal.path(), &overlap);

    assert_eq!(journal.load().unwrap(), vec![valid]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn role_and_presentation_combinations_are_validated() {
    let root = test_root("role-presentation");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let good_prompt = prompt("u1", "u1", 1, "question");
    let good_final = final_record("a1", "u1", 2, "answer");
    journal.append(&good_prompt).unwrap();
    journal.append(&good_final).unwrap();

    let mut styled_prompt = good_prompt.clone();
    styled_prompt.presentation = PersistedPresentation::Fallback;
    assert!(journal.append(&styled_prompt).is_err());
    let mut plain_final = good_final.clone();
    plain_final.presentation = PersistedPresentation::Plain;
    assert!(journal.append(&plain_final).is_err());

    for forbidden in ["reasoning", "interaction", "working", "tool", "tool_result"] {
        let mut value = serde_json::to_value(&good_prompt).unwrap();
        value["role"] = serde_json::Value::String(forbidden.into());
        append_json_line(journal.path(), &value);
    }

    assert_eq!(journal.load().unwrap(), vec![good_prompt, good_final]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn attachment_record_keeps_only_id_and_control_free_label() {
    let message = Message {
        stable_id: "u1".into(),
        text: "with image".into(),
        presentation: MessagePresentation::Plain,
        attachments: vec![Attachment {
            id: "image-1".into(),
            display: "screen\n\u{1b}]52;secret\u{7}.png".into(),
            native_path: Some("/private/tmp/hidden/screen.png".into()),
        }],
        timestamp_ms: Some(1),
    };

    let record = VisibleHistoryRecord::prompt(&message, 1).unwrap();
    assert_eq!(record.attachments.len(), 1);
    assert_eq!(record.attachments[0].id, "image-1");
    assert!(
        record.attachments[0]
            .display
            .chars()
            .all(|ch| !ch.is_control())
    );
    let serialized = serde_json::to_string(&record).unwrap();
    assert!(!serialized.contains("native_path"));
    assert!(!serialized.contains("/private/tmp"));
    assert!(!serialized.contains("secret"));
}

#[test]
fn compact_native_prompt_record_never_contains_hidden_paste_body() {
    let hidden = "private-log-line\n".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste(&hidden);
    let submission = editor.take_editor_submission();
    let expected = submission.display_text.clone();
    let mut app = AppState {
        session_id: "session-1".into(),
        ..AppState::default()
    };
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission,
        attachments: vec![],
        at_ms: 1,
    });
    assert!(app.drain_history_upserts().is_empty());

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        hidden,
        Some(2),
    )));
    let queued = app.drain_history_upserts();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].text, expected);
    assert!(queued[0].text.contains("[Pasted Content ·"));
    assert!(
        !serde_json::to_string(&queued[0])
            .unwrap()
            .contains("private-log-line")
    );
}

#[test]
fn asynchronous_writer_drop_flushes_latest_upsert_for_every_key() {
    let root = test_root("writer-drop");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let writer = HistoryWriter::spawn(journal.clone());
    let started = std::time::Instant::now();
    writer.queue(prompt("u1", "u1", 1, &"x".repeat(1_000_000)));
    writer.queue(prompt("u1", "u1", 1, "latest"));
    writer.queue(final_record("a1", "u1", 2, "answer"));
    assert!(started.elapsed() < std::time::Duration::from_millis(50));
    drop(writer);

    let loaded = journal.load().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].text, "latest");
    assert_eq!(loaded[1].text, "answer");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_attachment_shape_has_no_native_path_field() {
    let attachment = VisibleAttachment {
        id: "i1".into(),
        display: "screen.png".into(),
    };
    assert_eq!(
        serde_json::to_value(attachment).unwrap(),
        serde_json::json!({"id":"i1","display":"screen.png"})
    );
}

#[test]
fn concurrent_independent_journals_append_complete_json_lines() {
    let root = test_root("concurrent-append");
    let _ = std::fs::remove_dir_all(&root);
    let workers = 8_u64;
    let records_per_worker = 80_u64;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(workers as usize));
    let handles = (0..workers)
        .map(|worker| {
            let root = root.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
                barrier.wait();
                for index in 0..records_per_worker {
                    let id = format!("worker-{worker}-record-{index}");
                    let text = format!("{id}-{}", "x".repeat(32_000 + worker as usize));
                    journal
                        .append(&prompt(
                            &id,
                            &id,
                            worker * records_per_worker + index,
                            &text,
                        ))
                        .unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }

    let bytes = std::fs::read(
        HistoryJournal::at(&root, "w1:p1", "session-1")
            .unwrap()
            .path(),
    )
    .unwrap();
    let lines = bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    assert_eq!(lines.last(), Some(&&[][..]));
    assert_eq!(lines.len() - 1, (workers * records_per_worker) as usize);
    for line in &lines[..lines.len() - 1] {
        let record: VisibleHistoryRecord = serde_json::from_slice(line).unwrap();
        record.validate().unwrap();
    }
    assert_eq!(
        HistoryJournal::at(&root, "w1:p1", "session-1")
            .unwrap()
            .load()
            .unwrap()
            .len(),
        (workers * records_per_worker) as usize
    );
    std::fs::remove_dir_all(root).unwrap();
}
