# Compact Paste and Sticky Prompt Hierarchy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep large pasted bodies compact in the composer and prompt history without changing the text sent to Codex or Claude, clearly distinguish user prompts from final answers, and pin the first two wrapped prompt rows while its answer scrolls.

**Architecture:** Represent editor input as serializable text and large-paste chunks with separate display and submission projections. Carry the lossless submission only through the optimistic delivery path, persist only range/count/fingerprint metadata after native reconciliation, and apply that metadata when replaying provider transcripts. Replace the flat history builder with a render-only visual-row document so role styling, scrolling, and two-row sticky push-off all use one Unicode-aware geometry model.

**Tech Stack:** Rust 1.85+, Ratatui 0.29, Crossterm, Serde/Serde JSON, `unicode-width`, standard-library hashing/threads/channels, existing synthetic tests and fake Herdr socket.

---

## File structure

- Create `src/paste.rs`: own the `1_000`-character threshold, marker formatting, paste ranges, deterministic fingerprinting, native marker-count extraction, and persisted compact-history overrides.
- Modify `src/lib.rs`: export the focused `paste` module.
- Modify `src/editor.rs`: replace the single `String` buffer with serializable chunks, keep a cached display projection, treat large-paste tokens atomically, and return a lossless `EditorSubmission`.
- Modify `src/model.rs`: let optimistic deliveries retain their complete text, editor recovery snapshot, and paste ranges until reconciliation.
- Modify `src/app.rs`: reconcile full provider text or native compact markers, keep display-safe prompt text, restore chunked drafts after send failure, and maintain compact-history overrides.
- Modify `src/state.rs`: persist versioned chunked drafts and compact-history metadata through the existing non-blocking writer while accepting legacy string drafts.
- Modify `src/ui/mod.rs`: load/save chunked drafts, submit the complete projection, render the compact projection, and flush newly reconciled display metadata.
- Modify `src/ui/render.rs`: render compact composer text, role-aware history rows, explicit Unicode-aware wrapping, and sticky prompt geometry.
- Modify `tests/editor.rs`: cover the threshold, multiple paste tokens, lossless submission, Unicode counts, cursor skipping, and whole-token deletion.
- Modify `tests/app_state.rs`: cover optimistic compact display, full/native-marker reconciliation, replay overrides, and lossless failure recovery.
- Modify `tests/toggle_state.rs`: cover versioned draft persistence, legacy migration, metadata privacy, and writer coalescing.
- Modify `tests/ui_render.rs`: cover compact composer rendering, visible role hierarchy, wrapping, sticky behavior, next-prompt push-off, image-only prompts, and constrained viewports.
- Modify `tests/codex_parser.rs` and `tests/claude_parser.rs`: lock native compact-marker pass-through behavior.
- Modify `tests/transport_status.rs`: prove the complete large-paste payload, rather than its display marker, reaches `agent.prompt`.
- Modify `README.md`: document compact paste semantics, prompt/answer hierarchy, sticky context, and metadata privacy.

No new crate is needed. Do not add regex, Unicode segmentation, hashing, async-runtime, or persistence dependencies.

### Task 1: Add compact-paste policy and a lossless chunked editor

**Files:**
- Create: `src/paste.rs`
- Modify: `src/lib.rs`
- Modify: `src/editor.rs`
- Test: `tests/editor.rs`

**Required skill checkpoints:**
- Invoke `superpowers:test-driven-development` before editing production code.
- Invoke `superpowers:requesting-code-review` after the task's coding batch.
- Invoke `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Write failing threshold and submission tests**

Replace the old two-megabyte assertion with tests that distinguish display text from submitted text:

```rust
use herdr_simple_prompts::editor::{Editor, EditorCommand, Key, map_key, staged_image_path};
use herdr_simple_prompts::paste::{LARGE_PASTE_CHARS, large_paste_marker};

#[test]
fn paste_below_threshold_remains_directly_editable() {
    let mut editor = Editor::default();
    let pasted = "я".repeat(LARGE_PASTE_CHARS - 1);

    editor.insert_paste(&pasted);

    assert_eq!(editor.display_text(), pasted);
    assert_eq!(editor.submission_text(), pasted);
}

#[test]
fn large_paste_is_compact_but_submission_is_lossless() {
    let mut editor = Editor::default();
    let pasted = format!("first\n{}\nlast", "я".repeat(1_000_000));
    let count = pasted.chars().count();

    editor.insert_paste(&pasted);

    assert_eq!(editor.display_text(), large_paste_marker(count));
    assert_eq!(editor.submission_text(), pasted);
    let submission = editor.take_editor_submission();
    assert_eq!(submission.complete_text, pasted);
    assert_eq!(submission.display_text, large_paste_marker(count));
    assert_eq!(submission.paste_ranges.len(), 1);
    assert!(editor.is_empty());
}
```

- [ ] **Step 2: Write failing atomic-token tests**

Add exact coverage for multiple paste segments and editing at token boundaries:

```rust
#[test]
fn multiple_large_pastes_keep_order_and_delete_as_atomic_tokens() {
    let first = "a".repeat(LARGE_PASTE_CHARS);
    let second = "界".repeat(LARGE_PASTE_CHARS + 1);
    let mut editor = Editor::default();
    editor.insert_char('>');
    editor.insert_paste(&first);
    editor.insert_char('|');
    editor.insert_paste(&second);
    editor.insert_char('<');

    assert_eq!(
        editor.display_text(),
        format!(
            ">{}|{}<",
            large_paste_marker(first.chars().count()),
            large_paste_marker(second.chars().count()),
        )
    );
    assert_eq!(editor.submission_text(), format!(">{first}|{second}<"));

    let snapshot = editor.snapshot();
    let mut restored = Editor::default();
    restored.replace_snapshot(snapshot);
    assert_eq!(restored.display_text(), editor.display_text());
    assert_eq!(restored.submission_text(), editor.submission_text());

    editor.move_left();
    editor.backspace();

    assert_eq!(
        editor.display_text(),
        format!(">{}|<", large_paste_marker(first.chars().count()))
    );
    assert_eq!(editor.submission_text(), format!(">{first}|<"));
    assert!(editor.display_text().is_char_boundary(editor.display_cursor_byte()));
}

#[test]
fn cursor_never_enters_a_compact_paste_marker() {
    let mut editor = Editor::default();
    editor.insert_char('a');
    editor.insert_paste(&"x".repeat(LARGE_PASTE_CHARS));
    editor.insert_char('z');
    editor.move_left();
    let after_token = editor.display_cursor_byte();
    editor.move_left();
    let before_token = editor.display_cursor_byte();

    assert_eq!(after_token - before_token, large_paste_marker(LARGE_PASTE_CHARS).len());
}
```

- [ ] **Step 3: Run the editor tests and verify RED**

Run:

```bash
cargo test --test editor
```

Expected: compilation fails because `paste`, `display_text`, `submission_text`, `EditorSubmission`, and chunk-aware movement do not exist.

- [ ] **Step 4: Implement the compact-paste policy module**

Create `src/paste.rs` with these public contracts:

```rust
use serde::{Deserialize, Serialize};

pub const LARGE_PASTE_CHARS: usize = 1_000;

pub fn large_paste_marker(character_count: usize) -> String {
    format!("[Pasted Content · {character_count} chars]")
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PasteRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub character_count: usize,
}

pub fn fingerprint(text: &str) -> u64 {
    text.as_bytes().iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

pub fn marker_counts(text: &str) -> Vec<usize> {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .filter_map(|parts| {
            (parts[1].trim_matches(|c: char| !c.is_ascii_alphabetic()) == "chars")
                .then(|| parts[0].trim_matches(|c: char| !c.is_ascii_digit()).parse().ok())
                .flatten()
        })
        .collect()
}

pub fn canonicalize_compact_markers(text: &str) -> String {
    let mut output = text.to_owned();
    for count in marker_counts(text) {
        let canonical = large_paste_marker(count);
        for variant in [
            format!("[Pasted Content {count} chars]"),
            format!("[Pasted Content · {count} chars]"),
        ] {
            output = output.replace(&variant, &canonical);
        }
    }
    output
}
```

Export it from `src/lib.rs` with `pub mod paste;`. Keep marker parsing
dependency-free and add a private unit test proving both `[Pasted Content · 1000
chars]` and `[Pasted Content 1000 chars]` canonicalize to the dotted local
marker without changing their surrounding prompt text.

- [ ] **Step 5: Replace the editor string with serializable chunks**

In `src/editor.rs`, introduce the exact public data flow:

```rust
use crate::paste::{LARGE_PASTE_CHARS, PasteRange, large_paste_marker};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EditorChunk {
    Text(String),
    LargePaste {
        source_text: String,
        character_count: usize,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct EditorSnapshot {
    pub chunks: Vec<EditorChunk>,
}

impl EditorSnapshot {
    pub fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            chunks: (!text.is_empty())
                .then_some(EditorChunk::Text(text))
                .into_iter()
                .collect(),
        }
    }

    pub fn submission_text(&self) -> String {
        self.chunks.iter().fold(String::new(), |mut output, chunk| {
            output.push_str(match chunk {
                EditorChunk::Text(text) => text,
                EditorChunk::LargePaste { source_text, .. } => source_text,
            });
            output
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorSubmission {
    pub complete_text: String,
    pub display_text: String,
    pub recovery: EditorSnapshot,
    pub paste_ranges: Vec<PasteRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EditorAtom {
    Character(char),
    LargePaste {
        source_text: String,
        character_count: usize,
    },
}

#[derive(Clone, Default)]
pub struct Editor {
    atoms: Vec<EditorAtom>,
    source: String,
    display: String,
    cursor_atom: usize,
    preferred_column: Option<usize>,
}
```

Keep the existing `text()`, `cursor_byte()`, and `take_submission()` contracts as
lossless compatibility projections until Task 3 switches the UI explicitly to
the compact projection. Add these methods and retain the existing key mapping
and staged-image helper:

```rust
pub fn display_text(&self) -> &str { &self.display }
pub fn text(&self) -> &str { &self.source }
pub fn submission_text(&self) -> &str { &self.source }
pub fn is_empty(&self) -> bool { self.source.is_empty() }
pub fn cursor_byte(&self) -> usize { self.source_cursor_byte(self.cursor_atom) }
pub fn display_cursor_byte(&self) -> usize { self.display_cursor_byte_at(self.cursor_atom) }
pub fn snapshot(&self) -> EditorSnapshot { EditorSnapshot::from_atoms(&self.atoms) }

pub fn replace(&mut self, text: impl Into<String>) {
    self.replace_snapshot(EditorSnapshot::plain(text));
}

pub fn replace_snapshot(&mut self, snapshot: EditorSnapshot) {
    self.atoms = snapshot.into_atoms();
    self.cursor_atom = self.atoms.len();
    self.preferred_column = None;
    self.rebuild_projections();
}

pub fn clear(&mut self) {
    self.atoms.clear();
    self.source.clear();
    self.display.clear();
    self.cursor_atom = 0;
    self.preferred_column = None;
}

pub fn take_submission(&mut self) -> String {
    self.take_editor_submission().complete_text
}

pub fn take_editor_submission(&mut self) -> EditorSubmission {
    let submission = EditorSubmission {
        complete_text: self.source.clone(),
        display_text: self.display.clone(),
        recovery: self.snapshot(),
        paste_ranges: self.paste_ranges(),
    };
    self.clear();
    submission
}
```

Use character atoms internally so a large paste is naturally one cursor item and
ordinary Unicode input is never split. Implement the conversion and projection
helpers exactly once:

```rust
impl EditorSnapshot {
    fn into_atoms(self) -> Vec<EditorAtom> {
        self.chunks.into_iter().flat_map(|chunk| match chunk {
            EditorChunk::Text(text) => text.chars().map(EditorAtom::Character).collect(),
            EditorChunk::LargePaste { source_text, character_count } => vec![
                EditorAtom::LargePaste { source_text, character_count },
            ],
        }).collect()
    }

    fn from_atoms(atoms: &[EditorAtom]) -> Self {
        let mut chunks = Vec::new();
        let mut text = String::new();
        for atom in atoms {
            match atom {
                EditorAtom::Character(character) => text.push(*character),
                EditorAtom::LargePaste { source_text, character_count } => {
                    if !text.is_empty() {
                        chunks.push(EditorChunk::Text(std::mem::take(&mut text)));
                    }
                    chunks.push(EditorChunk::LargePaste {
                        source_text: source_text.clone(),
                        character_count: *character_count,
                    });
                }
            }
        }
        if !text.is_empty() { chunks.push(EditorChunk::Text(text)); }
        Self { chunks }
    }
}

fn rebuild_projections(&mut self) {
    self.source.clear();
    self.display.clear();
    for atom in &self.atoms {
        match atom {
            EditorAtom::Character(character) => {
                self.source.push(*character);
                self.display.push(*character);
            }
            EditorAtom::LargePaste { source_text, character_count } => {
                self.source.push_str(source_text);
                self.display.push_str(&large_paste_marker(*character_count));
            }
        }
    }
}

fn paste_ranges(&self) -> Vec<PasteRange> {
    let mut byte = 0;
    let mut ranges = Vec::new();
    for atom in &self.atoms {
        match atom {
            EditorAtom::Character(character) => byte += character.len_utf8(),
            EditorAtom::LargePaste { source_text, character_count } => {
                let start_byte = byte;
                byte += source_text.len();
                ranges.push(PasteRange { start_byte, end_byte: byte, character_count: *character_count });
            }
        }
    }
    ranges
}
```

`insert_char` inserts one `Character` at `cursor_atom`. `insert_paste` inserts
one `LargePaste` atom when `text.chars().count() >= LARGE_PASTE_CHARS`, otherwise
it inserts `text.chars().map(EditorAtom::Character)`. Both advance `cursor_atom`
by the inserted atom count and rebuild the two cached projections.

`move_left`/`move_right` decrement/increment `cursor_atom`; `backspace` removes
`atoms[cursor_atom - 1]`; `delete` removes `atoms[cursor_atom]`. Therefore a
large paste moves and deletes atomically without special marker-string edits.
For vertical/home/end movement, calculate the target byte in `self.display`
with the existing display-column helpers, then select the closest value from
`(0..=atoms.len()).map(display_cursor_byte_at)`; ties select the earlier
boundary. This prevents the cursor from entering a marker or UTF-8 scalar.

- [ ] **Step 6: Run Task 1 tests and verify GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test editor
cargo test paste::
```

Expected: all commands exit 0; the million-character source survives exactly while the display contains only its marker.

- [ ] **Step 7: Review, verify, and commit Task 1**

Invoke `superpowers:requesting-code-review`, address every Critical/Important finding, invoke `superpowers:verification-before-completion`, rerun Step 6, then commit:

```bash
git add src/lib.rs src/paste.rs src/editor.rs tests/editor.rs
git commit -m "model compact large paste input"
```

### Task 2: Persist chunked drafts and privacy-safe history overrides

**Files:**
- Modify: `src/paste.rs`
- Modify: `src/state.rs`
- Test: `tests/toggle_state.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle before production edits.
- Invoke `superpowers:requesting-code-review` after the task's coding batch.
- Invoke `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Write failing persistence and privacy tests**

Update the state tests to use snapshots and add a legacy fixture:

```rust
use herdr_simple_prompts::editor::{Editor, EditorSnapshot};
use herdr_simple_prompts::paste::{CompactPromptOverride, LARGE_PASTE_CHARS};

#[test]
fn chunked_draft_reopens_with_full_source_behind_compact_token() {
    let directory = test_state_directory("chunked-draft");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let pasted = "draft-log-line\n".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste(&pasted);

    store.save_editor_draft("w1:p1", &editor.snapshot(), &[], &[]).unwrap();

    let state = store.load_draft("w1:p1").unwrap();
    let mut restored = Editor::default();
    restored.replace_snapshot(state.editor);
    assert_eq!(restored.submission_text(), pasted);
    assert!(restored.display_text().contains("Pasted Content"));
    assert!(!restored.display_text().contains("draft-log-line"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn compact_history_metadata_does_not_persist_pasted_body() {
    let directory = test_state_directory("history-metadata");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let pasted = "private-log-line\n".repeat(LARGE_PASTE_CHARS);
    let mut editor = Editor::default();
    editor.insert_paste(&pasted);
    let submission = editor.take_editor_submission();
    let summary = CompactPromptOverride::new(
        "session-1",
        "native-1",
        &submission.complete_text,
        submission.paste_ranges.clone(),
    );

    store.save_editor_draft("w1:p1", &EditorSnapshot::default(), &[], &[summary]).unwrap();

    let serialized = std::fs::read_to_string(directory.join("draft-w1_p1.json")).unwrap();
    assert!(!serialized.contains("private-log-line"));
    let state = store.load_draft("w1:p1").unwrap();
    assert_eq!(state.prompt_displays.len(), 1);
    assert_eq!(
        state.prompt_displays[0].compact_text(&submission.complete_text).unwrap(),
        submission.display_text,
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_string_draft_loads_as_plain_editor_snapshot() {
    let directory = test_state_directory("legacy-draft");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("draft-w1_p1.json"),
        r#"{"text":"old\ndraft","attachments":[]}"#,
    ).unwrap();

    let state = StateStore::at(&directory).load_draft("w1:p1").unwrap();

    assert_eq!(state.editor, EditorSnapshot::plain("old\ndraft"));
    assert!(state.prompt_displays.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}
```

Extract the repeated temporary-directory construction with this exact helper and
keep cleanup explicit at each test end:

```rust
fn test_state_directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "herdr-simple-prompts-{label}-{}",
        std::process::id(),
    ))
}
```

- [ ] **Step 2: Run focused state tests and verify RED**

Run:

```bash
cargo test --test toggle_state chunked_draft_reopens_with_full_source_behind_compact_token -- --exact
cargo test --test toggle_state compact_history_metadata_does_not_persist_pasted_body -- --exact
cargo test --test toggle_state legacy_string_draft_loads_as_plain_editor_snapshot -- --exact
```

Expected: compilation fails because chunked draft and `CompactPromptOverride` persistence do not exist.

- [ ] **Step 3: Implement privacy-safe prompt display metadata**

Extend `src/paste.rs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CompactPromptOverride {
    pub session_id: String,
    pub stable_id: String,
    complete_len: usize,
    fingerprint: u64,
    paste_ranges: Vec<PasteRange>,
}

impl CompactPromptOverride {
    pub fn new(
        session_id: impl Into<String>,
        stable_id: impl Into<String>,
        complete_text: &str,
        paste_ranges: Vec<PasteRange>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            stable_id: stable_id.into(),
            complete_len: complete_text.len(),
            fingerprint: fingerprint(complete_text),
            paste_ranges,
        }
    }

    pub fn compact_text(&self, complete_text: &str) -> Option<String> {
        if complete_text.len() != self.complete_len || fingerprint(complete_text) != self.fingerprint {
            return None;
        }
        let mut output = complete_text.to_owned();
        for range in self.paste_ranges.iter().rev() {
            if range.start_byte > range.end_byte
                || range.end_byte > output.len()
                || !output.is_char_boundary(range.start_byte)
                || !output.is_char_boundary(range.end_byte)
            {
                return None;
            }
            output.replace_range(
                range.start_byte..range.end_byte,
                &large_paste_marker(range.character_count),
            );
        }
        Some(output)
    }
}
```

Keep all fields except `session_id` and `stable_id` private. Metadata must contain length, range, count, and deterministic fingerprint only—never pasted source bytes or a copied display prompt.

- [ ] **Step 4: Version the persisted draft format without losing old drafts**

Change `DraftState` and the writer snapshot in `src/state.rs`:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DraftState {
    pub text: String,
    pub editor: EditorSnapshot,
    pub attachments: Vec<Attachment>,
    pub prompt_displays: Vec<CompactPromptOverride>,
}

#[derive(Deserialize, Serialize)]
struct PersistedDraft {
    version: u8,
    editor: EditorSnapshot,
    attachments: Vec<PersistedAttachment>,
    #[serde(default)]
    prompt_displays: Vec<CompactPromptOverride>,
}

#[derive(Deserialize)]
struct LegacyDraft {
    text: String,
    attachments: Vec<PersistedAttachment>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ReadDraft {
    Current(PersistedDraft),
    Legacy(LegacyDraft),
}
```

Add `save_editor_draft`, which accepts `&EditorSnapshot`, attachments, and
prompt displays and always writes `version: 2`. Keep the existing `save_draft`
signature as a compatibility wrapper that calls `save_editor_draft` with
`EditorSnapshot::plain(text)` and an empty override list. `load_draft` converts
`LegacyDraft.text` with `EditorSnapshot::plain` and fills `DraftState.text` from
the snapshot's complete source projection. Keep invalid-file quarantine, `0600`
file mode, and omission of attachment `native_path`.

Keep `DraftWriter::queue(text, attachments)` as a compatibility wrapper and add
`queue_editor`; both feed the same `DraftSnapshot`:

```rust
pub fn queue_editor(
    &self,
    editor: EditorSnapshot,
    attachments: Vec<Attachment>,
    prompt_displays: Vec<CompactPromptOverride>,
) {
    let (lock, ready) = &*self.slot;
    let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.pending = Some(DraftSnapshot { editor, attachments, prompt_displays });
    ready.notify_one();
}

pub fn queue(&self, text: String, attachments: Vec<Attachment>) {
    self.queue_editor(EditorSnapshot::plain(text), attachments, Vec::new());
}
```

In the worker, replace the old `save_draft` call with:

```rust
store.save_editor_draft(
    &pane_id,
    &snapshot.editor,
    &snapshot.attachments,
    &snapshot.prompt_displays,
)
```

The worker continues coalescing the newest complete snapshot and performs
filesystem I/O off the UI caller.

- [ ] **Step 5: Run Task 2 tests and verify GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test toggle_state
```

Expected: all state tests pass; legacy drafts load; current empty drafts can persist compact metadata without containing the sent pasted body.

- [ ] **Step 6: Review, verify, and commit Task 2**

Invoke `superpowers:requesting-code-review`, address Critical/Important findings, invoke `superpowers:verification-before-completion`, rerun Step 5, then commit:

```bash
git add src/paste.rs src/state.rs tests/toggle_state.rs
git commit -m "persist compact paste state"
```

### Task 3: Reconcile compact optimistic prompts with native history

**Files:**
- Modify: `src/model.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/render.rs`
- Test: `tests/app_state.rs`
- Test: `tests/codex_parser.rs`
- Test: `tests/claude_parser.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/transport_status.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle before production edits.
- Invoke `superpowers:requesting-code-review` after the task's coding batch.
- Invoke `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Add provider marker characterization tests**

These are deliberately GREEN characterization tests: adapters already preserve visible user text and must not start parsing or expanding native markers.

```rust
#[test]
fn preserves_native_compact_paste_marker_exactly() {
    let mut adapter = CodexAdapter;
    let event = adapter.ingest_line(
        1,
        r#"{"type":"event_msg","payload":{"type":"user_message","item_id":"u1","message":"inspect\n[Pasted Content 1000 chars]"}}"#,
    ).unwrap().unwrap();
    assert!(matches!(event, ConversationEvent::User(message)
        if message.text == "inspect\n[Pasted Content 1000 chars]"));
}
```

Add the Claude equivalent with a top-level `user` record and string
`message.content`:

```rust
#[test]
fn preserves_native_compact_paste_marker_exactly() {
    let mut adapter = ClaudeAdapter::default();
    let events = adapter.ingest_line(
        1,
        r#"{"type":"user","uuid":"u1","message":{"content":"inspect\n[Pasted Content 1000 chars]"}}"#,
    ).unwrap();
    assert!(matches!(&events[0], ConversationEvent::User(message)
        if message.text == "inspect\n[Pasted Content 1000 chars]"));
}
```

Run both focused tests and expect PASS before changing reducer code.

- [ ] **Step 2: Write failing optimistic reconciliation tests**

Add a helper that builds a real `EditorSubmission`, then cover full transcript text, provider markers, replay, and failure:

```rust
fn compact_submission(source: &str) -> EditorSubmission {
    let mut editor = Editor::default();
    editor.insert_paste(source);
    editor.take_editor_submission()
}

fn plain_submission(source: &str) -> EditorSubmission {
    let mut editor = Editor::default();
    editor.replace(source);
    editor.take_editor_submission()
}

#[test]
fn full_native_text_reconciles_without_expanding_compact_prompt() {
    let source = "log\n".repeat(1_000);
    let submission = compact_submission(&source);
    let expected_display = submission.display_text.clone();
    let mut app = AppState { session_id: "session-1".into(), ..AppState::default() };
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission,
        attachments: vec![],
        at_ms: 100,
    });

    app.apply(AppEvent::NativeUser(Message::text("native-1", source, Some(120))));

    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.text, expected_display);
    assert_eq!(app.turns[0].delivery, Delivery::Native);
    assert_eq!(app.prompt_displays.len(), 1);
}

#[test]
fn native_marker_reconciles_and_is_preserved() {
    let source = "x".repeat(1_000);
    let mut app = AppState { session_id: "session-1".into(), ..AppState::default() };
    app.apply(AppEvent::PromptSubmitted {
        local_id: "local-1".into(),
        submission: compact_submission(&source),
        attachments: vec![],
        at_ms: 100,
    });

    app.apply(AppEvent::NativeUser(Message::text(
        "native-1",
        "[Pasted Content 1000 chars]",
        Some(120),
    )));

    assert_eq!(app.turns.len(), 1);
    assert_eq!(app.turns[0].prompt.text, "[Pasted Content 1000 chars]");
}

#[test]
fn replay_applies_persisted_override_to_full_provider_text() {
    let source = "secret\n".repeat(1_000);
    let submission = compact_submission(&source);
    let expected_display = submission.display_text.clone();
    let summary = CompactPromptOverride::new(
        "session-1",
        "native-1",
        &submission.complete_text,
        submission.paste_ranges.clone(),
    );
    let mut app = AppState {
        session_id: "session-1".into(),
        prompt_displays: vec![summary],
        ..AppState::default()
    };

    app.apply(AppEvent::NativeUser(Message::text("native-1", source, Some(10))));

    assert_eq!(app.turns[0].prompt.text, expected_display);
    assert!(!app.turns[0].prompt.text.contains("secret"));
}
```

Update every existing `PromptSubmitted` constructor to use
`submission: plain_submission(text)`. Rewrite the failure assertion without
using a moved value:

```rust
let mut failed_editor = Editor::default();
failed_editor.insert_paste(&"a".repeat(LARGE_PASTE_CHARS));
failed_editor.insert_char('|');
failed_editor.insert_paste(&"界".repeat(LARGE_PASTE_CHARS + 1));
let submission = failed_editor.take_editor_submission();
let expected_recovery = submission.recovery.clone();
app.apply(AppEvent::PromptSubmitted {
    local_id: "local-1".into(),
    submission,
    attachments: vec![],
    at_ms: 1,
});
app.apply(AppEvent::SendFailed {
    local_id: "local-1".into(),
    reason: "pane closed".into(),
});
assert_eq!(app.draft, expected_recovery);
let mut restored = Editor::default();
restored.replace_snapshot(app.draft.clone());
assert_eq!(restored.display_text().matches("Pasted Content").count(), 2);
```

- [ ] **Step 3: Run reducer tests and verify RED**

Run:

```bash
cargo test --test app_state
```

Expected: compilation fails because `PromptSubmitted` still accepts one string and optimistic delivery has no recovery/metadata fields.

- [ ] **Step 4: Extend the optimistic delivery model**

In `src/model.rs`, import editor/paste types and change only the optimistic variant:

```rust
Optimistic {
    local_id: String,
    submitted_at_ms: u64,
    complete_text: String,
    recovery: EditorSnapshot,
    paste_ranges: Vec<PasteRange>,
}
```

Keep `Message.text` display-safe. Do not add complete text to `Message`, native turns, final answers, or persisted conversation history.

In `src/app.rs`, change `PromptSubmitted.text` to `submission: EditorSubmission`, add `session_id: String`, change `draft` to `EditorSnapshot`, and add `prompt_displays: Vec<CompactPromptOverride>`.

- [ ] **Step 5: Implement full/native-marker reconciliation**

On `PromptSubmitted`, put `submission.display_text` in `Message.text` and the lossless fields in `Delivery::Optimistic`.

Before inserting a native user message, look up an override with matching `session_id` and `stable_id`; if `compact_text(&message.text)` succeeds, replace only the display text.

For current optimistic deliveries, match in submission order with the existing attachment/time guards and this predicate:

```rust
normalized_text(complete_text) == normalized_text(&message.text)
    || normalized_text(&turn.prompt.text) == normalized_text(&message.text)
    || (!paste_ranges.is_empty()
        && normalized_text(&canonicalize_compact_markers(&message.text))
            == normalized_text(&turn.prompt.text))
```

After a match:

1. if normalized provider text equals normalized `complete_text`, retain the
   optimistic compact `turn.prompt.text` even if the pasted body happens to
   contain a phrase such as `1000 chars`;
2. otherwise the match came from a compact representation, so preserve the
   provider's marker text exactly;
3. replace the id/timestamp/attachments with native values;
4. append or replace `CompactPromptOverride::new(session_id, native_id, complete_text, paste_ranges)`;
5. set delivery to `Native`, which drops the complete text and recovery snapshot.

On `SendFailed`, copy `recovery` into `app.draft`, restore attachments, and mark the turn failed. Keep timestamp-window and transcript-replay behavior unchanged.

- [ ] **Step 6: Write failing composer and complete-transport tests**

Add to `tests/ui_render.rs`:

```rust
#[test]
fn composer_shows_large_paste_marker_instead_of_log_body() {
    let mut editor = Editor::default();
    editor.insert_char('>');
    editor.insert_paste(&"private-log-line\n".repeat(1_000));
    editor.insert_char('<');

    let rendered = render_to_string(&AppState::default(), &editor, 80, 12);

    assert!(rendered.contains("Pasted Content"));
    assert!(rendered.contains("chars"));
    assert!(!rendered.contains("private-log-line"));
    assert!(rendered.contains('>'));
    assert!(rendered.contains('<'));
}
```

Add to `tests/transport_status.rs`:

```rust
#[test]
fn submit_forwards_complete_large_paste_without_display_marker() {
    let source = "line\n".repeat(1_000);
    let fake = support::ScriptedHerdr::start(vec![
        agent_result("s1"),
        json!({"type":"agent_prompted"}),
    ]);
    let client = HerdrClient::connect(fake.socket_path()).unwrap();
    let transport = AgentTransport::new(client, identity("s1"));

    transport.submit(&source).unwrap();

    let requests = fake.requests();
    assert_eq!(requests[1]["method"], "agent.prompt");
    assert_eq!(requests[1]["params"]["text"], source);
    assert!(!requests[1]["params"]["text"].as_str().unwrap().contains("Pasted Content"));
}
```

Run both focused tests. Expected: the composer test is RED until rendering uses
the display projection; the transport characterization may already be GREEN and
remains the end-to-end payload guard.

- [ ] **Step 7: Wire chunked state and both submission projections through the UI**

In `run_from_env`, load only summaries for the current native session and restore
the editor snapshot:

```rust
let mut prompt_displays = draft.prompt_displays;
prompt_displays.retain(|summary| summary.session_id == identity.session_id);
editor.replace_snapshot(draft.editor);
let mut app = AppState {
    session_id: identity.session_id.clone(),
    prompt_displays,
    agent_status: identity.status,
    working_since: identity.status.is_working().then(Instant::now),
    draft_attachments: draft.attachments,
    ..AppState::default()
};
```

Every draft save becomes:

```rust
writer.queue_editor(
    editor.snapshot(),
    app.draft_attachments.clone(),
    app.prompt_displays.clone(),
);
```

Aggregate whether follower reconciliation changed `app.prompt_displays`; return
`DraftChange::Immediate` when it did so the background writer persists the new
stable-id override. Queue once after initial replay to discard summaries from a
different session.

Change the Enter branch to send the complete text while the reducer receives
the whole submission object:

```rust
if editor.submission_text().trim().is_empty() && app.draft_attachments.is_empty() {
    return Ok(DraftChange::None);
}
let submission = editor.take_editor_submission();
let complete_text = submission.complete_text.clone();
app.send_error = None;
let attachments = app.draft_attachments.clone();
let local_id = format!("local-{}", *local_sequence);
*local_sequence += 1;
app.apply(AppEvent::PromptSubmitted {
    local_id: local_id.clone(),
    submission,
    attachments,
    at_ms: now_ms(),
});
if let Err(error) = runtime.submit(local_id.clone(), complete_text) {
    app.apply(AppEvent::SendFailed { local_id, reason: error.to_string() });
    editor.replace_snapshot(app.draft.clone());
    app.send_error = Some(error.to_string());
}
```

Use `replace_snapshot(app.draft.clone())` in the asynchronous
`RuntimeEvent::Submitted` failure branch too. Keep `UiRuntime::submit` and
`AgentTransport::submit` string-only.

- [ ] **Step 8: Render only the compact editor projection**

In `src/ui/render.rs`, bind `let editor_text = editor.display_text();` once and
use it for composer height, placeholder selection, line creation, and
`editor_visual_cursor`. Pass `editor.display_cursor_byte()` to the cursor helper.
Do not call `submission_text()` or legacy `text()` from rendering.

- [ ] **Step 9: Run Task 3 tests and verify GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test codex_parser preserves_native_compact_paste_marker_exactly -- --exact
cargo test --test claude_parser preserves_native_compact_paste_marker_exactly -- --exact
cargo test --test app_state
cargo test --test toggle_state
cargo test --test ui_render composer_shows_large_paste_marker_instead_of_log_body -- --exact
cargo test --test transport_status
```

Expected: all commands pass; compact local prompts reconcile once, replay stays
compact, failed sends restore chunks, rendering hides the log body, transport
sends it exactly, and lossless text disappears from delivery after reconciliation.

- [ ] **Step 10: Review, verify, and commit Task 3**

Invoke `superpowers:requesting-code-review`, address Critical/Important findings,
invoke `superpowers:verification-before-completion`, rerun Step 9, then commit:

```bash
git add src/model.rs src/app.rs src/ui/mod.rs src/ui/render.rs tests/app_state.rs tests/codex_parser.rs tests/claude_parser.rs tests/ui_render.rs tests/transport_status.rs
git commit -m "reconcile compact prompt history"
```

### Task 4: Introduce styled history sections and role hierarchy

**Files:**
- Modify: `src/ui/render.rs`
- Test: `tests/ui_render.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle before production edits.
- Invoke `superpowers:requesting-code-review` after the task's coding batch.
- Invoke `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Write a failing role-hierarchy test**

Add a buffer-aware test helper and assert semantic labels plus a raised prompt surface:

```rust
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

fn rendered_buffer(app: &AppState, width: u16, height: u16) -> Buffer {
    herdr_simple_prompts::ui::render::render_to_buffer(
        app,
        &Editor::default(),
        width,
        height,
    )
}

#[test]
fn prompt_band_and_answer_label_distinguish_roles_without_color_only() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text("u1", "check dns", Some(1))));
    app.apply(AppEvent::NativeFinal(Message::text("a1", "zone is pending", Some(2))));

    let rendered = render_to_string(&app, &Editor::default(), 50, 14);
    let buffer = rendered_buffer(&app, 50, 14);

    assert!(rendered.contains("YOU  check dns"));
    assert!(rendered.contains("ANSWER"));
    let user_row = (0..14)
        .find(|&row| buffer[(0, row)].symbol() == "Y")
        .expect("YOU row");
    assert_eq!(buffer[(0, user_row)].style().bg, Some(Color::DarkGray));
    assert!(buffer[(0, user_row)].style().add_modifier.contains(Modifier::BOLD));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --test ui_render prompt_band_and_answer_label_distinguish_roles_without_color_only -- --exact
```

Expected: FAIL because `render_to_buffer`, `YOU`, and `ANSWER` do not exist.

- [ ] **Step 3: Add the render-only history document types**

Replace `history_text` with focused private types:

```rust
const PROMPT_BG: Color = Color::DarkGray;
const PROMPT_FG: Color = Color::White;
const ANSWER_FG: Color = Color::Green;

#[derive(Clone, Debug)]
struct HistoryRow { line: Line<'static> }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PromptSection {
    start_row: u16,
    prompt_rows: u16,
    end_row: u16,
}

#[derive(Clone, Debug, Default)]
struct HistoryDocument {
    rows: Vec<HistoryRow>,
    prompts: Vec<PromptSection>,
}

impl HistoryDocument {
    fn text(&self) -> Text<'static> {
        Text::from(self.rows.iter().map(|row| row.line.clone()).collect::<Vec<_>>())
    }
}
```

Build one prompt band per turn. Its first row begins with bold `YOU  `;
continuation rows begin with spaces of the same display width. If prompt text is
empty and attachments exist, place the first `[Image #1] ...` placeholder on
the `YOU` row so an image-only prompt has useful sticky context; render any
remaining attachments on continuation rows. Apply named-color
background/foreground styles to the full line, including attachment and
failed-delivery rows. Render final answers unboxed with a bold green `ANSWER`
label and normal-background text. Do not hard-code RGB values.

- [ ] **Step 4: Add `render_to_buffer` and retain string snapshots**

Factor the existing test-backend code:

```rust
pub fn render_to_buffer(app: &AppState, editor: &Editor, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, app, editor)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    buffer
}
```

Make `render_to_string` serialize this returned buffer row-by-row exactly as before.

- [ ] **Step 5: Run, review, verify, and commit Task 4**

Run the focused test and `cargo test --test ui_render`. Invoke `superpowers:requesting-code-review`, fix Critical/Important findings, invoke `superpowers:verification-before-completion`, then run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test ui_render
```

Expected: all commands exit 0. Commit:

```bash
git add src/ui/render.rs tests/ui_render.rs
git commit -m "render distinct prompt and answer roles"
```

### Task 5: Build one explicit Unicode-aware visual-row model

**Files:**
- Modify: `src/ui/render.rs`
- Test: private unit tests in `src/ui/render.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle before production edits.
- Invoke `superpowers:requesting-code-review` after the task's coding batch.
- Invoke `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Write failing visual-row unit tests**

Extend the private `src/ui/render.rs` test module so it can inspect the same
render-only document used by the frame:

```rust
#[test]
fn wrap_line_preserves_unicode_width_and_style() {
    let style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let rows = wrap_line(&Line::styled("界界a", style), 4);

    let symbols = rows.iter().map(|line| {
        line.spans.iter().fold(String::new(), |mut output, span| {
            output.push_str(span.content.as_ref());
            output
        })
    }).collect::<Vec<_>>();

    assert_eq!(symbols, ["界界", "a"]);
    assert!(rows.iter().flat_map(|line| &line.spans)
        .all(|span| span.style.fg == Some(Color::Cyan)));
}

#[test]
fn history_document_records_wrapped_prompt_rows_after_explicit_newline() {
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(Message::text(
        "u1", "界界界\nnext", Some(1),
    )));

    let document = build_history_document(&app, 10);

    assert_eq!(document.prompts[0].prompt_rows, 3);
    assert_eq!(document.prompts[0].start_row, 0);
    for row in &document.rows[..3] {
        let width = row.line.spans.iter()
            .map(|span| unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
            .sum::<usize>();
        assert_eq!(width, 10);
        assert_eq!(row.line.spans.last().unwrap().style.bg, Some(PROMPT_BG));
    }
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test ui::render::tests::wrap_line_preserves_unicode_width_and_style -- --exact
cargo test ui::render::tests::history_document_records_wrapped_prompt_rows_after_explicit_newline -- --exact
```

Expected: compilation or assertions fail because explicit styled-row wrapping and
width-aware document construction do not exist.

- [ ] **Step 3: Wrap styled lines once before geometry and rendering**

Add the wrapper with one source of truth for symbols and styles:

```rust
fn wrap_line(line: &Line<'static>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut rows = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0_usize;

    for span in &line.spans {
        for grapheme in span.styled_graphemes(line.style) {
            let grapheme_width = unicode_width::UnicodeWidthStr::width(grapheme.symbol);
            if !current.is_empty()
                && current_width.saturating_add(grapheme_width) > width
            {
                rows.push(Line::from(std::mem::take(&mut current)));
                current_width = 0;
            }
            current.push(Span::styled(grapheme.symbol.to_owned(), grapheme.style));
            current_width = current_width.saturating_add(grapheme_width);
            if current_width >= width {
                rows.push(Line::from(std::mem::take(&mut current)));
                current_width = 0;
            }
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(Line::from(current));
    }
    rows
}
```

Change the Task 4 builder to
`build_history_document(app: &AppState, width: u16)`. Flatten every styled
source line through `wrap_line`, then calculate each `PromptSection` from the
resulting row indices. Pad each wrapped prompt-band row to `width` with a
trailing `Span::styled(" ".repeat(missing_width), prompt_style)` so the neutral
background reaches the viewport's right edge; do not pad answer rows. Render
with `Paragraph` without `.wrap(...)`. Delete
`wrapped_history_height`; use:

```rust
let history_height = u16::try_from(document.rows.len()).unwrap_or(u16::MAX);
```

Keep `wrapped_text_height` for the compact composer only.

- [ ] **Step 4: Run, review, verify, and commit Task 5**

Run both focused unit tests and `cargo test --test ui_render`. Invoke
`superpowers:requesting-code-review`, fix Critical/Important findings, invoke
`superpowers:verification-before-completion`, then run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test ui_render
```

Expected: all commands exit 0. Commit:

```bash
git add src/ui/render.rs
git commit -m "model unicode aware history rows"
```

### Task 6: Add two-row sticky selection and next-prompt push-off

**Files:**
- Modify: `src/ui/render.rs`
- Test: `tests/ui_render.rs`

**Required skill checkpoints:**
- Continue the active `superpowers:test-driven-development` cycle before production edits.
- Invoke `superpowers:requesting-code-review` after the task's coding batch.
- Invoke `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Write failing sticky-context and boundary tests**

Add this local helper:

```rust
fn app_with_turns(turns: &[(&str, &str)]) -> AppState {
    let mut app = AppState::default();
    for (index, (prompt, answer)) in turns.iter().enumerate() {
        let timestamp = u64::try_from(index * 2).unwrap();
        app.apply(AppEvent::NativeUser(Message::text(
            format!("u{index}"), *prompt, Some(timestamp),
        )));
        app.apply(AppEvent::NativeFinal(Message::text(
            format!("a{index}"), *answer, Some(timestamp + 1),
        )));
    }
    app
}
```

Add the normal, Unicode, image-only, provider-marker, push-off, and constrained
viewport guards:

```rust
#[test]
fn sticky_prompt_is_not_duplicated_while_naturally_visible() {
    let app = app_with_turns(&[("first prompt", "short answer")]);
    let rendered = render_to_string(&app, &Editor::default(), 40, 14);
    assert_eq!(rendered.matches("first prompt").count(), 1);
}

#[test]
fn next_prompt_pushes_previous_context_one_row_at_a_time() {
    let mut app = app_with_turns(&[
        ("old prompt first row\nold prompt second row", "answer line\n"),
        ("next prompt", "next answer"),
    ]);

    app.scroll_from_bottom = 3;
    let before = render_to_string(&app, &Editor::default(), 38, 12);
    assert!(before.lines().next().unwrap().contains("old prompt first row"));

    app.scroll_from_bottom = 2;
    let pushed_one = render_to_string(&app, &Editor::default(), 38, 12);
    assert!(!pushed_one.contains("old prompt first row"));
    assert!(pushed_one.lines().next().unwrap().contains("old prompt second row"));

    app.scroll_from_bottom = 1;
    let next = render_to_string(&app, &Editor::default(), 38, 12);
    assert!(!next.contains("old prompt second row"));
    assert!(next.lines().next().unwrap().contains("next prompt"));
}

#[test]
fn tiny_history_keeps_one_scroll_row_below_sticky_context() {
    let mut app = app_with_turns(&[("first\nsecond\nthird", &"answer\n".repeat(10))]);
    app.scroll_from_bottom = 4;
    let rendered = render_to_string(&app, &Editor::default(), 30, 7);
    assert!(rendered.contains("answer"));
}

#[test]
fn prompt_context_uses_two_unicode_aware_visual_rows() {
    let mut app = app_with_turns(&[(
        "界界界界 alpha beta gamma delta",
        &"answer\n".repeat(18),
    )]);
    app.scroll_from_bottom = 8;

    let rendered = render_to_string(&app, &Editor::default(), 24, 12);

    assert!(rendered.lines().next().unwrap().contains("YOU"));
    assert_eq!(rendered.lines().take(2).filter(|line| !line.trim().is_empty()).count(), 2);
}

#[test]
fn image_only_prompt_uses_first_attachment_as_sticky_context() {
    let mut prompt = Message::text("u1", "", Some(1));
    prompt.attachments.push(Attachment {
        id: "image-1".into(),
        display: "screen.png".into(),
        native_path: None,
    });
    let mut app = AppState::default();
    app.apply(AppEvent::NativeUser(prompt));
    app.apply(AppEvent::NativeFinal(Message::text("a1", "long answer\n".repeat(12), Some(2))));
    app.scroll_from_bottom = 5;

    let rendered = render_to_string(&app, &Editor::default(), 30, 12);

    assert!(rendered.lines().next().unwrap().contains("[Image #1] screen.png"));
}

#[test]
fn native_large_paste_marker_stays_compact_in_sticky_context() {
    let mut app = app_with_turns(&[(
        "inspect this\n[Pasted Content 1000 chars]",
        &"answer\n".repeat(12),
    )]);
    app.scroll_from_bottom = 5;

    let rendered = render_to_string(&app, &Editor::default(), 40, 12);

    assert!(rendered.lines().take(2).any(|line| line.contains("1000 chars")));
    assert!(!rendered.contains("first log line"));
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run each test with `cargo test --test ui_render <name> -- --exact`. The
no-duplication guard may pass already; Unicode/image/provider context, push-off,
and constrained-viewport tests provide RED because no sticky overlay exists.

- [ ] **Step 3: Implement sticky geometry as a pure function**

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StickyPrompt {
    section_index: usize,
    first_prompt_row: u16,
    visible_rows: u16,
}

fn sticky_prompt(
    document: &HistoryDocument,
    viewport_top: u16,
    viewport_height: u16,
) -> Option<StickyPrompt> {
    let capacity = viewport_height.saturating_sub(1).min(2);
    if capacity == 0 { return None; }
    let section_index = document.prompts.iter().rposition(|section| {
        section.start_row < viewport_top && viewport_top < section.end_row
    })?;
    let section = document.prompts[section_index];
    let natural_rows = section.prompt_rows.min(capacity);
    let push = document.prompts.get(section_index + 1)
        .map(|next| viewport_top.saturating_add(natural_rows).saturating_sub(next.start_row))
        .unwrap_or(0)
        .min(natural_rows);
    (natural_rows > push).then_some(StickyPrompt {
        section_index,
        first_prompt_row: push,
        visible_rows: natural_rows - push,
    })
}
```

Validate inequality directions against the RED tests: no sticky copy while the prompt is naturally visible; show at most two prompt rows after its start scrolls out; shrink to one old row when the next prompt is one row from the top; show none when the next prompt reaches the boundary.

- [ ] **Step 4: Overlay only the top history cells**

Calculate `viewport_top` with existing bottom-offset semantics and render normal
history first. Then overlay only the selected rows:

```rust
if let Some(sticky) = sticky_prompt(&document, viewport_top, areas[0].height) {
    let sticky_area = Rect::new(
        areas[0].x,
        areas[0].y,
        areas[0].width,
        sticky.visible_rows,
    );
    let section = document.prompts[sticky.section_index];
    let start = usize::from(section.start_row + sticky.first_prompt_row);
    let end = start + usize::from(sticky.visible_rows);
    let lines = document.rows[start..end]
        .iter()
        .map(|row| row.line.clone())
        .collect::<Vec<_>>();
    frame.render_widget(Clear, sticky_area);
    frame.render_widget(Paragraph::new(Text::from(lines)), sticky_area);
}
```

Never clear the working line, error line, composer, or footer. Preserve at least
one scrolling history row through the pure function's `viewport_height - 1`
capacity.

- [ ] **Step 5: Run, review, verify, and commit Task 6**

Run all sticky tests and `cargo test --test ui_render`. Invoke `superpowers:requesting-code-review`, fix Critical/Important findings, invoke `superpowers:verification-before-completion`, then run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test ui_render
```

Expected: all commands exit 0. Commit:

```bash
git add src/ui/render.rs tests/ui_render.rs
git commit -m "pin current prompt while scrolling"
```

### Task 7: Document, verify, and smoke-test the rebuilt plugin

**Files:**
- Modify: `README.md`
- Verify: all source and test files changed in Tasks 1-6

**Required skill checkpoints:**
- Tester-oriented checkpoint does not apply to the documentation edit itself; behavior is covered by Tasks 1-6.
- Invoke `superpowers:requesting-code-review` for the complete feature diff.
- Invoke `superpowers:verification-before-completion` before any completion claim.

- [ ] **Step 1: Update public behavior and privacy documentation**

Replace the current generic paste paragraph with:

```markdown
Pastes below 1,000 characters remain directly editable. A paste of 1,000
characters or more appears as `[Pasted Content · N chars]` in the composer and
simple history, while Codex or Claude receives the complete original text.
Compact paste tokens move and delete atomically; failed sends restore the full
draft.
```

Add feature bullets:

```markdown
- User prompts render as labeled full-width bands; final answers remain unboxed
  and carry an `ANSWER` label.
- While an answer scrolls, the first two visual rows of its prompt stay at the
  top. The next prompt pushes that context out one row at a time.
```

In Privacy and state, list compact-paste ranges/counts/fingerprints and state that sent pasted bodies are not copied; only an unsent draft can contain the original paste. Document that old transcript prompts stay compact only when the provider recorded a native marker or the plugin has metadata from their submission.

- [ ] **Step 2: Run the complete verification gate**

Invoke `superpowers:verification-before-completion`, then run exactly:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
git diff --check
```

Expected: every command exits 0; the suite has zero failed tests; `target/release/herdr-simple-prompts` is rebuilt.

- [ ] **Step 3: Perform the live Herdr smoke matrix**

With a supported idle Codex or Claude pane focused:

1. press `prefix+m` and confirm Simple Prompts opens;
2. paste a harmless 1,000-character string and confirm the composer shows one `1000 chars` token rather than the body;
3. type visible text before and after the token, move across it, and confirm one cursor step skips the token;
4. submit and confirm the prompt appears immediately above `Working` with the same compact token;
5. return to the native pane and confirm the native agent received the original text, not the marker;
6. reopen Simple Prompts and confirm the sent prompt remains compact;
7. confirm user prompts have a full-width `YOU` band and final answers have an unboxed `ANSWER` label;
8. scroll through a long answer and confirm exactly two wrapped prompt rows stick;
9. continue until the following prompt pushes the old rows out one at a time;
10. resize narrower and confirm Unicode wrapping and sticky ownership recompute;
11. return to the bottom and confirm live auto-scroll resumes;
12. press `prefix+m` and confirm the overlay closes without changing the native session.

Use only synthetic paste contents. Do not include real transcript text in logs, screenshots, commits, or reports.

- [ ] **Step 4: Request complete feature review**

Invoke `superpowers:requesting-code-review` with the range from commit `7b6dcc0` through current `HEAD`, the approved design sections `Message hierarchy and sticky prompt context`, `Composer behavior`, `Editor and compact-paste model`, `Optimistic reconciliation`, and `Persistence and privacy`, plus verification output. Fix every Critical/Important issue and rerun Step 2 after any code change.

- [ ] **Step 5: Commit documentation and final review adjustments**

```bash
git add README.md
git commit -m "document compact sticky prompt view"
```

If review changed source/tests, add only those reviewed files to the same final-adjustment commit and name the commit for the actual correction.

- [ ] **Step 6: Confirm repository and installed runtime state**

Run:

```bash
git status --short --branch
herdr plugin list --json
herdr plugin log list --plugin herdr.simple-prompts --limit 5
```

Expected: repository is clean; `herdr.simple-prompts` is enabled and linked to this source tree; the final smoke invocation has no failed action log.
