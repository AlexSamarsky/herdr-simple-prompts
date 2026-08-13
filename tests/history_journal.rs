use herdr_simple_prompts::app::{AppEvent, AppState};
use herdr_simple_prompts::editor::Editor;
use herdr_simple_prompts::history::{
    HistoryJournal, HistoryWriter, PersistedPresentation, VisibleAttachment, VisibleHistoryRecord,
    VisibleRole,
};
use herdr_simple_prompts::model::{Attachment, Message};
use herdr_simple_prompts::paste::{LARGE_PASTE_CHARS, fingerprint};
use herdr_simple_prompts::state::StateStore;
use herdr_simple_prompts::style::{
    AnsiColor, MessagePresentation, StyleModifiers, StyleRun, StyledText,
};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;

fn test_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "herdr-simple-prompts-history-{label}-{}",
        std::process::id()
    ))
}

fn prompt(id: &str, turn_id: &str, order: u64, text: &str) -> VisibleHistoryRecord {
    VisibleHistoryRecord {
        version: 2,
        role: VisibleRole::Prompt,
        stable_id: id.into(),
        turn_id: turn_id.into(),
        order,
        text: text.into(),
        attachments: Vec::new(),
        timestamp_ms: Some(order),
        text_fingerprint: fingerprint(text),
        presentation: PersistedPresentation::Plain,
        rendered_text: None,
        rendered_text_fingerprint: None,
    }
}

fn final_record(id: &str, turn_id: &str, order: u64, text: &str) -> VisibleHistoryRecord {
    VisibleHistoryRecord {
        version: 2,
        role: VisibleRole::Final,
        stable_id: id.into(),
        turn_id: turn_id.into(),
        order,
        text: text.into(),
        attachments: Vec::new(),
        timestamp_ms: Some(order),
        text_fingerprint: fingerprint(text),
        presentation: PersistedPresentation::Fallback,
        rendered_text: None,
        rendered_text_fingerprint: None,
    }
}

fn set_native(record: &mut VisibleHistoryRecord, rendered: &str) {
    record.presentation = PersistedPresentation::NativeAnsi(vec![native_run(rendered)]);
    record.rendered_text = Some(rendered.into());
    record.rendered_text_fingerprint = Some(fingerprint(rendered));
}

fn legacy_final(id: &str, turn_id: &str, order: u64, text: &str) -> VisibleHistoryRecord {
    let mut record = final_record(id, turn_id, order, text);
    record.version = 1;
    record
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
    set_native(&mut native, "answer");
    journal.append(&fallback).unwrap();
    journal.append(&native).unwrap();

    assert_eq!(journal.load().unwrap(), vec![native]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_native_roundtrip_and_hydration_preserve_canonical_and_rendered_text() {
    let root = test_root("v2-native-roundtrip");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let canonical = "# **Answer** [docs](https://example.test)";
    let rendered = "Answer docs";
    let prompt_record = prompt("u1", "u1", 1, "question");
    let mut final_record = final_record("a1", "u1", 2, canonical);
    set_native(&mut final_record, rendered);

    journal.append(&prompt_record).unwrap();
    journal.append(&final_record).unwrap();
    let loaded = journal.load().unwrap();
    assert_eq!(loaded, vec![prompt_record, final_record.clone()]);

    let mut app = AppState::default();
    app.hydrate_visible_history(loaded);
    let restored = app.turns[0].final_answer.as_ref().unwrap();
    assert_eq!(restored.text, canonical);
    assert_eq!(
        restored.presentation,
        MessagePresentation::NativeAnsi(StyledText {
            text: rendered.into(),
            runs: vec![native_run(rendered)],
        })
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_native_rendered_integrity_is_validated_independently_of_canonical_text() {
    let canonical = "canonical transcript text that is much longer than rendered";
    let mut valid = final_record("a1", "u1", 2, canonical);
    set_native(&mut valid, "short");
    assert!(valid.validate().is_ok());

    let mut missing_rendered = valid.clone();
    missing_rendered.rendered_text = None;
    assert!(missing_rendered.validate().is_err());
    let mut missing_fingerprint = valid.clone();
    missing_fingerprint.rendered_text_fingerprint = None;
    assert!(missing_fingerprint.validate().is_err());

    let mut fingerprint_mismatch = valid.clone();
    fingerprint_mismatch.rendered_text_fingerprint = Some(fingerprint("different"));
    assert!(fingerprint_mismatch.validate().is_err());

    let mut rendered_control = valid.clone();
    rendered_control.rendered_text = Some("unsafe\u{1b}".into());
    rendered_control.rendered_text_fingerprint = Some(fingerprint("unsafe\u{1b}"));
    rendered_control.presentation = PersistedPresentation::NativeAnsi(Vec::new());
    assert!(rendered_control.validate().is_err());

    let mut canonical_relative = valid.clone();
    canonical_relative.presentation = PersistedPresentation::NativeAnsi(vec![StyleRun {
        start_byte: 0,
        end_byte: 20,
        ..native_run("short")
    }]);
    assert!(canonical_relative.validate().is_err());

    let mut fallback_rendered = final_record("a2", "u1", 3, "fallback");
    fallback_rendered.rendered_text = Some("not allowed".into());
    fallback_rendered.rendered_text_fingerprint = Some(fingerprint("not allowed"));
    assert!(fallback_rendered.validate().is_err());

    let mut prompt_rendered = prompt("u2", "u2", 4, "question");
    prompt_rendered.rendered_text = Some("not allowed".into());
    prompt_rendered.rendered_text_fingerprint = Some(fingerprint("not allowed"));
    assert!(prompt_rendered.validate().is_err());
}

#[test]
fn v1_native_records_restore_only_when_projection_is_byte_identical() {
    let runs = vec![native_run("plain answer")];
    let mut plain = legacy_final("a1", "u1", 2, "plain answer");
    plain.presentation = PersistedPresentation::NativeAnsi(runs.clone());
    assert!(plain.validate().is_ok());

    let markdown = "**bold answer**";
    let mut projected = legacy_final("a2", "u2", 4, markdown);
    projected.presentation = PersistedPresentation::NativeAnsi(vec![native_run(markdown)]);
    assert!(projected.validate().is_ok());

    let mut app = AppState::default();
    let mut first_prompt = prompt("u1", "u1", 1, "first");
    first_prompt.version = 1;
    let mut second_prompt = prompt("u2", "u2", 3, "second");
    second_prompt.version = 1;
    app.hydrate_visible_history(vec![first_prompt, plain, second_prompt, projected]);

    assert_eq!(
        app.turns[0].final_answer.as_ref().unwrap().presentation,
        MessagePresentation::NativeAnsi(StyledText {
            text: "plain answer".into(),
            runs,
        })
    );
    assert_eq!(
        app.turns[1].final_answer.as_ref().unwrap().presentation,
        MessagePresentation::MarkdownFallback
    );
}

#[test]
fn v1_records_reject_v2_rendered_fields() {
    let mut legacy = legacy_final("a1", "u1", 2, "plain answer");
    legacy.presentation = PersistedPresentation::NativeAnsi(vec![native_run("plain answer")]);
    legacy.rendered_text = Some("plain answer".into());
    legacy.rendered_text_fingerprint = Some(fingerprint("plain answer"));

    assert!(legacy.validate().is_err());
}

#[test]
fn v1_native_records_reject_control_bytes_before_hydration() {
    let text = "unsafe\u{1b}";
    let mut legacy = legacy_final("a1", "u1", 2, text);
    legacy.presentation = PersistedPresentation::NativeAnsi(Vec::new());

    assert!(legacy.validate().is_err());
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
fn append_repairs_an_incomplete_tail_before_writing_the_next_record() {
    let root = test_root("repair-incomplete");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let first = prompt("u1", "u1", 1, "safe");
    let next = prompt("u3", "u3", 3, "after recovery");
    journal.append(&first).unwrap();
    let mut partial = serde_json::to_vec(&prompt("u2", "u2", 2, "partial")).unwrap();
    partial.truncate(partial.len() / 2);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(journal.path())
        .unwrap()
        .write_all(&partial)
        .unwrap();

    journal.append(&next).unwrap();

    assert_eq!(journal.load().unwrap(), vec![first, next]);
    assert_eq!(std::fs::read(journal.path()).unwrap().last(), Some(&b'\n'));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn append_rejects_symlinked_history_components_and_journal_files() {
    let root = test_root("append-symlink");
    let external = test_root("append-symlink-external");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    let external_file = external.join("keep.jsonl");
    std::fs::write(&external_file, b"external\n").unwrap();
    symlink(&external, root.join("history")).unwrap();
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();

    assert!(journal.append(&prompt("u1", "u1", 1, "question")).is_err());
    assert_eq!(std::fs::read(&external_file).unwrap(), b"external\n");

    std::fs::remove_file(root.join("history")).unwrap();
    std::fs::create_dir_all(root.join("history/w1_p1")).unwrap();
    symlink(&external_file, journal.path()).unwrap();
    assert!(journal.append(&prompt("u2", "u2", 2, "answer")).is_err());
    assert_eq!(std::fs::read(&external_file).unwrap(), b"external\n");
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(external).unwrap();
}

#[test]
fn load_rejects_a_symlinked_state_root() {
    let root = test_root("load-root-symlink");
    let external = test_root("load-root-symlink-external");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(external.join("history/w1_p1")).unwrap();
    std::fs::write(
        external.join("history/w1_p1/session-1.jsonl"),
        b"external\n",
    )
    .unwrap();
    symlink(&external, &root).unwrap();
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();

    assert!(journal.load().is_err());
    assert_eq!(
        std::fs::read(external.join("history/w1_p1/session-1.jsonl")).unwrap(),
        b"external\n"
    );
    std::fs::remove_file(root).unwrap();
    std::fs::remove_dir_all(external).unwrap();
}

#[test]
fn load_rejects_symlinked_history_and_pane_ancestors() {
    let root = test_root("load-ancestor-symlink");
    let external = test_root("load-ancestor-symlink-external");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&external);
    std::fs::create_dir_all(external.join("w1_p1")).unwrap();
    std::fs::write(external.join("w1_p1/session-1.jsonl"), b"external\n").unwrap();
    std::fs::create_dir_all(&root).unwrap();
    symlink(&external, root.join("history")).unwrap();
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    assert!(journal.load().is_err());

    std::fs::remove_file(root.join("history")).unwrap();
    std::fs::create_dir_all(root.join("history")).unwrap();
    symlink(external.join("w1_p1"), root.join("history/w1_p1")).unwrap();
    assert!(journal.load().is_err());
    assert_eq!(
        std::fs::read(external.join("w1_p1/session-1.jsonl")).unwrap(),
        b"external\n"
    );
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(external).unwrap();
}

#[test]
fn invalid_upserts_do_not_poison_the_last_valid_record() {
    let root = test_root("invalid");
    let _ = std::fs::remove_dir_all(&root);
    let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
    let valid = final_record("a1", "u1", 2, "a界b");
    journal.append(&valid).unwrap();

    let mut invalid_version = valid.clone();
    invalid_version.version = 3;
    append_json_line(journal.path(), &invalid_version);
    let mut invalid_fingerprint = valid.clone();
    invalid_fingerprint.text_fingerprint ^= 1;
    append_json_line(journal.path(), &invalid_fingerprint);
    let mut split_scalar = valid.clone();
    set_native(&mut split_scalar, "a界b");
    split_scalar.presentation = PersistedPresentation::NativeAnsi(vec![StyleRun {
        start_byte: 1,
        end_byte: 2,
        ..native_run("a界b")
    }]);
    append_json_line(journal.path(), &split_scalar);
    let mut overlap = valid.clone();
    set_native(&mut overlap, "a界b");
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

#[test]
fn pane_state_cleanup_removes_only_the_exact_pane_history_namespace() {
    let root = test_root("pane-cleanup");
    let _ = std::fs::remove_dir_all(&root);
    let store = StateStore::at(&root);
    let target = store.history_journal("w1:p1", "session-1").unwrap();
    let sibling = store.history_journal("w1:p10", "session-10").unwrap();
    target.append(&prompt("u1", "u1", 1, "target")).unwrap();
    sibling.append(&prompt("u10", "u10", 1, "sibling")).unwrap();

    store.remove_pane_state("w1:p1").unwrap();

    assert!(!target.path().exists());
    assert!(!root.join("history/w1_p1").exists());
    assert!(sibling.path().exists());
    assert_eq!(sibling.load().unwrap().len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
