use std::path::PathBuf;

use unicode_width::UnicodeWidthChar;

use crate::model::Attachment;
use crate::paste::{LARGE_PASTE_CHARS, PasteRange, large_paste_marker};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Enter,
    ShiftEnter,
    Ctrl(char),
    Character(char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorCommand {
    Submit,
    Newline,
    Insert(char),
    None,
}

pub fn map_key(key: Key) -> EditorCommand {
    match key {
        Key::Enter => EditorCommand::Submit,
        Key::ShiftEnter | Key::Ctrl('j') => EditorCommand::Newline,
        Key::Character(character) => EditorCommand::Insert(character),
        Key::Ctrl(_) => EditorCommand::None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EditorChunk {
    Text(String),
    LargePaste {
        source_text: String,
        character_count: usize,
    },
    /// An image the agent is holding. It contributes nothing to the prompt
    /// text — the image itself lives in the native composer — but it occupies a
    /// place in the line so it can be moved through like any other content.
    Attachment {
        id: String,
        display: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EditorSnapshot {
    pub chunks: Vec<EditorChunk>,
}

impl EditorSnapshot {
    pub fn attachments(&self) -> Vec<Attachment> {
        self.chunks
            .iter()
            .filter_map(|chunk| match chunk {
                EditorChunk::Attachment { id, display } => Some(Attachment {
                    id: id.clone(),
                    display: display.clone(),
                    native_path: None,
                }),
                EditorChunk::Text(_) | EditorChunk::LargePaste { .. } => None,
            })
            .collect()
    }

    pub fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        if text.is_empty() {
            Self::default()
        } else {
            Self {
                chunks: vec![EditorChunk::Text(text)],
            }
        }
    }

    pub fn submission_text(&self) -> String {
        let capacity = self
            .chunks
            .iter()
            .map(|chunk| match chunk {
                EditorChunk::Text(text) => text.len(),
                EditorChunk::LargePaste { source_text, .. } => source_text.len(),
                EditorChunk::Attachment { .. } => 0,
            })
            .sum();
        let mut submission = String::with_capacity(capacity);
        for chunk in &self.chunks {
            match chunk {
                EditorChunk::Text(text) => submission.push_str(text),
                EditorChunk::LargePaste { source_text, .. } => submission.push_str(source_text),
                EditorChunk::Attachment { .. } => {}
            }
        }
        submission
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorSubmission {
    pub complete_text: String,
    pub display_text: String,
    pub recovery: EditorSnapshot,
    pub paste_ranges: Vec<PasteRange>,
}

#[derive(Clone)]
enum EditorAtom {
    Character(char),
    /// The gap beside an image: a place for the cursor to stand, and nothing
    /// more. It is drawn, so the markers do not run together, and it is not
    /// part of the prompt, so standing there costs the text nothing.
    Gap,
    LargePaste {
        source_text: String,
        character_count: usize,
    },
    Attachment(Attachment),
}

#[derive(Clone)]
pub struct Editor {
    atoms: Vec<EditorAtom>,
    cursor: usize,
    source_text: String,
    display_text: String,
    source_boundaries: Vec<usize>,
    display_boundaries: Vec<usize>,
    preferred_column: Option<usize>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            atoms: Vec::new(),
            cursor: 0,
            source_text: String::new(),
            display_text: String::new(),
            source_boundaries: vec![0],
            display_boundaries: vec![0],
            preferred_column: None,
        }
    }
}

impl Editor {
    pub fn text(&self) -> &str {
        self.submission_text()
    }

    pub fn submission_text(&self) -> &str {
        &self.source_text
    }

    pub fn display_text(&self) -> &str {
        &self.display_text
    }

    pub fn is_empty(&self) -> bool {
        self.source_text.is_empty()
    }

    /// Whether the draft holds nothing at all — not even an image marker.
    pub fn is_blank(&self) -> bool {
        self.atoms.is_empty()
    }

    pub fn cursor_byte(&self) -> usize {
        self.source_boundaries[self.cursor]
    }

    pub fn display_cursor_byte(&self) -> usize {
        self.display_boundaries[self.cursor]
    }

    pub fn insert_char(&mut self, character: char) {
        self.atoms
            .insert(self.cursor, EditorAtom::Character(character));
        self.cursor += 1;
        self.preferred_column = None;
        self.rebuild_projections();
    }

    /// Places an image marker at the cursor.
    ///
    /// The marker sits in the line the way the native composer shows it, rather
    /// than on a shelf above the input, and moves with the text around it.
    pub fn insert_attachment(&mut self, attachment: Attachment) {
        self.atoms
            .insert(self.cursor, EditorAtom::Attachment(attachment));
        self.cursor += 1;
        // The gap after a marker is a place of its own, so the cursor has
        // somewhere to stand between two images — as it does in the pane. Drawn
        // as part of the marker, that place did not exist and a step left
        // jumped straight from one picture to the previous one.
        self.atoms.insert(self.cursor, EditorAtom::Gap);
        self.cursor += 1;
        self.preferred_column = None;
        self.rebuild_projections();
    }

    /// Brings the markers in line with what the pane is actually holding.
    ///
    /// A marker is a claim about the native composer, and the composer can move
    /// on without us: an image that was submitted or cleared leaves a marker
    /// behind that stands for nothing. Left alone, the overlay keeps insisting
    /// the pane holds an image it does not, and guards its own input for as
    /// long as the draft survives.
    pub fn sync_attachments(&mut self, markers: &[usize]) {
        let mut seen = 0;
        self.atoms.retain_mut(|atom| match atom {
            EditorAtom::Attachment(attachment) => {
                let Some(marker) = markers.get(seen) else {
                    return false;
                };
                seen += 1;
                // The pane owns the name as well as the count: a marker whose
                // label went stale would later name the wrong picture, or none
                // at all, and the overlay would drop a chip the pane still holds.
                attachment.id = format!("native-image-{marker}");
                attachment.display = format!("Image #{marker}");
                true
            }
            EditorAtom::Character(_) | EditorAtom::LargePaste { .. } | EditorAtom::Gap => true,
        });
        // A gap belongs to the marker in front of it; one left behind by a
        // marker that has gone would show as a stray space.
        let mut kept: Vec<EditorAtom> = Vec::with_capacity(self.atoms.len());
        for atom in self.atoms.drain(..) {
            if matches!(atom, EditorAtom::Gap)
                && !matches!(kept.last(), Some(EditorAtom::Attachment(_)))
            {
                continue;
            }
            kept.push(atom);
        }
        self.atoms = kept;
        for marker in markers.iter().skip(seen) {
            self.atoms.push(EditorAtom::Attachment(Attachment {
                id: format!("native-image-{marker}"),
                display: format!("Image #{marker}"),
                native_path: None,
            }));
            self.atoms.push(EditorAtom::Gap);
        }
        self.cursor = self.cursor.min(self.atoms.len());
        self.preferred_column = None;
        self.rebuild_projections();
    }

    /// The image the cursor is standing on — the one shown marked, and so the
    /// one a person means when they press backspace.
    pub fn attachment_at_cursor(&self) -> Option<&Attachment> {
        match self.atoms.get(self.cursor) {
            Some(EditorAtom::Attachment(attachment)) => Some(attachment),
            _ => None,
        }
    }

    /// Where the image under the cursor sits in the shown text.
    ///
    /// The cursor stands *on* an image, not after it: an image just added is
    /// behind the cursor and stays plain, and stepping left onto it lights it
    /// up. Marking the one behind instead lit it the moment it appeared and
    /// again whenever the cursor moved past it, which reads as the wrong image
    /// being pointed at.
    pub fn attachment_span_at_cursor(&self) -> Option<(usize, usize)> {
        let index = self.cursor;
        if !matches!(self.atoms.get(index), Some(EditorAtom::Attachment(_))) {
            return None;
        }
        let start = *self.display_boundaries.get(index)?;
        let end = *self.display_boundaries.get(index + 1)?;
        (start < end).then_some((start, end))
    }

    /// The image the cursor has just passed, gap and all.
    ///
    /// Standing at the end of the line, a person pressing backspace means the
    /// image they can see, not the space beside it — the native composer takes
    /// the image there too. Without this the key ate the gap and then met the
    /// wall, and looked broken.
    pub fn attachment_behind_cursor(&self) -> Option<&Attachment> {
        let previous = self.cursor.checked_sub(1)?;
        match self.atoms.get(previous) {
            Some(EditorAtom::Attachment(attachment)) => Some(attachment),
            Some(EditorAtom::Gap) => match self.atoms.get(previous.checked_sub(1)?) {
                Some(EditorAtom::Attachment(attachment)) => Some(attachment),
                _ => None,
            },
            _ => None,
        }
    }

    /// Drops an image once the pane has confirmed it is gone there too.
    pub fn remove_attachment(&mut self, id: &str) {
        let Some(index) = self.atoms.iter().position(
            |atom| matches!(atom, EditorAtom::Attachment(attachment) if attachment.id == id),
        ) else {
            return;
        };
        self.atoms.remove(index);
        // The gap belonged to that image and goes with it.
        if matches!(self.atoms.get(index), Some(EditorAtom::Gap)) {
            self.atoms.remove(index);
        }
        self.cursor = self.cursor.min(index).min(self.atoms.len());
        // The caret is left standing in the gap the neighbours keep between
        // them, not on the picture that moved up into the place. A marked
        // picture says backspace is about to take that one, and taking this one
        // was the whole of what was asked for.
        if matches!(self.atoms.get(self.cursor), Some(EditorAtom::Attachment(_)))
            && matches!(
                self.cursor
                    .checked_sub(1)
                    .and_then(|gap| self.atoms.get(gap)),
                Some(EditorAtom::Gap)
            )
        {
            self.cursor -= 1;
        }
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn attachments(&self) -> Vec<Attachment> {
        self.atoms
            .iter()
            .filter_map(|atom| match atom {
                EditorAtom::Attachment(attachment) => Some(attachment.clone()),
                EditorAtom::Character(_) | EditorAtom::LargePaste { .. } | EditorAtom::Gap => None,
            })
            .collect()
    }

    pub fn insert_paste(&mut self, text: &str) {
        let character_count = text.chars().count();
        if character_count >= LARGE_PASTE_CHARS {
            self.atoms.insert(
                self.cursor,
                EditorAtom::LargePaste {
                    source_text: text.to_owned(),
                    character_count,
                },
            );
            self.cursor += 1;
        } else {
            let inserted_count = character_count;
            self.atoms.splice(
                self.cursor..self.cursor,
                text.chars().map(EditorAtom::Character),
            );
            self.cursor += inserted_count;
        }
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.preferred_column = None;
    }

    pub fn move_right(&mut self) {
        if self.cursor == self.atoms.len() {
            return;
        }
        self.cursor += 1;
        self.preferred_column = None;
    }

    pub fn move_word_left(&mut self) {
        self.cursor = self.word_start_before(self.cursor);
        self.preferred_column = None;
    }

    pub fn move_word_right(&mut self) {
        self.cursor = self.word_end_after(self.cursor);
        self.preferred_column = None;
    }

    pub fn delete_word_left(&mut self) {
        let start = self
            .word_start_before(self.cursor)
            .max(self.attachment_floor(self.cursor));
        if start == self.cursor {
            return;
        }
        self.atoms.drain(start..self.cursor);
        self.cursor = start;
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn delete_word_right(&mut self) {
        let end = self
            .word_end_after(self.cursor)
            .min(self.attachment_ceiling(self.cursor));
        if end == self.cursor {
            return;
        }
        self.atoms.drain(self.cursor..end);
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 || self.is_opaque_attachment(self.cursor - 1) {
            return;
        }
        self.cursor -= 1;
        self.atoms.remove(self.cursor);
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn delete(&mut self) {
        if self.cursor == self.atoms.len() || self.is_opaque_attachment(self.cursor) {
            return;
        }
        self.atoms.remove(self.cursor);
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn move_home(&mut self) {
        self.cursor = self.line_start_index();
        self.preferred_column = None;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.line_end_index();
        self.preferred_column = None;
    }

    pub fn delete_to_line_start(&mut self) {
        let start = self
            .line_start_index()
            .max(self.attachment_floor(self.cursor));
        if start == self.cursor {
            return;
        }
        self.atoms.drain(start..self.cursor);
        self.cursor = start;
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn delete_to_line_end(&mut self) {
        let end = self
            .line_end_index()
            .min(self.attachment_ceiling(self.cursor));
        if end == self.cursor {
            return;
        }
        self.atoms.drain(self.cursor..end);
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn move_document_start(&mut self) {
        self.cursor = 0;
        self.preferred_column = None;
    }

    pub fn move_document_end(&mut self) {
        self.cursor = self.atoms.len();
        self.preferred_column = None;
    }

    pub fn move_up(&mut self) {
        self.move_vertical(-1);
    }

    pub fn move_down(&mut self) {
        self.move_vertical(1);
    }

    pub fn clear(&mut self) {
        self.atoms.clear();
        self.cursor = 0;
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn replace(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.atoms = text.chars().map(EditorAtom::Character).collect();
        self.cursor = self.atoms.len();
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn snapshot(&self) -> EditorSnapshot {
        let mut chunks = Vec::new();
        let mut plain_text = String::new();

        for atom in &self.atoms {
            match atom {
                EditorAtom::Character(character) => plain_text.push(*character),
                EditorAtom::Attachment(attachment) => {
                    if !plain_text.is_empty() {
                        chunks.push(EditorChunk::Text(std::mem::take(&mut plain_text)));
                    }
                    chunks.push(EditorChunk::Attachment {
                        id: attachment.id.clone(),
                        display: attachment.display.clone(),
                    });
                }
                // The gap belongs to the marker beside it and is put back with
                // it, so it is not written down twice.
                EditorAtom::Gap => {}
                EditorAtom::LargePaste {
                    source_text,
                    character_count,
                } => {
                    if !plain_text.is_empty() {
                        chunks.push(EditorChunk::Text(std::mem::take(&mut plain_text)));
                    }
                    chunks.push(EditorChunk::LargePaste {
                        source_text: source_text.clone(),
                        character_count: *character_count,
                    });
                }
            }
        }
        if !plain_text.is_empty() {
            chunks.push(EditorChunk::Text(plain_text));
        }

        EditorSnapshot { chunks }
    }

    pub fn replace_snapshot(&mut self, snapshot: EditorSnapshot) {
        self.atoms.clear();
        for chunk in snapshot.chunks {
            match chunk {
                EditorChunk::Text(text) => {
                    self.atoms.extend(text.chars().map(EditorAtom::Character));
                }
                EditorChunk::LargePaste {
                    source_text,
                    character_count,
                } => self.atoms.push(EditorAtom::LargePaste {
                    source_text,
                    character_count,
                }),
                EditorChunk::Attachment { id, display } => {
                    self.atoms.push(EditorAtom::Attachment(Attachment {
                        id,
                        display,
                        native_path: None,
                    }));
                    self.atoms.push(EditorAtom::Gap);
                }
            }
        }
        self.cursor = self.atoms.len();
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn take_submission(&mut self) -> String {
        let complete_text = std::mem::take(&mut self.source_text);
        self.clear();
        complete_text
    }

    pub fn take_editor_submission(&mut self) -> EditorSubmission {
        let submission = EditorSubmission {
            complete_text: self.source_text.clone(),
            display_text: self.display_text.clone(),
            recovery: self.snapshot(),
            paste_ranges: self.paste_ranges(),
        };
        self.clear();
        submission
    }

    /// Word boundaries are counted in atoms, not bytes.
    ///
    /// A collapsed paste is a single atom and stands for a whole block of text,
    /// so it counts as one word: a word motion stops at its edge instead of
    /// stepping into a body the composer is not showing.
    fn line_start_index(&self) -> usize {
        let display_cursor = self.display_cursor_byte();
        let target = self.display_text[..display_cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        self.closest_display_boundary(target)
    }

    fn line_end_index(&self) -> usize {
        let display_cursor = self.display_cursor_byte();
        let target = self.display_text[display_cursor..]
            .find('\n')
            .map(|index| display_cursor + index)
            .unwrap_or(self.display_text.len());
        self.closest_display_boundary(target)
    }

    fn word_start_before(&self, cursor: usize) -> usize {
        let mut index = cursor;
        while index > 0 && self.is_whitespace(index - 1) {
            index -= 1;
        }
        if index > 0 && self.is_opaque(index - 1) {
            return index - 1;
        }
        while index > 0 && !self.is_whitespace(index - 1) && !self.is_opaque(index - 1) {
            index -= 1;
        }
        index
    }

    fn word_end_after(&self, cursor: usize) -> usize {
        let mut index = cursor;
        while index < self.atoms.len() && self.is_whitespace(index) {
            index += 1;
        }
        if index < self.atoms.len() && self.is_opaque(index) {
            return index + 1;
        }
        while index < self.atoms.len() && !self.is_whitespace(index) && !self.is_opaque(index) {
            index += 1;
        }
        index
    }

    fn is_whitespace(&self, index: usize) -> bool {
        matches!(
            self.atoms.get(index),
            Some(EditorAtom::Character(character)) if character.is_whitespace()
        )
    }

    /// Deletions stop at an image rather than passing through it.
    ///
    /// The marker stands for a picture the agent is holding, and removing it
    /// here has to remove it there — which is not wired up yet. Until it is,
    /// the marker is a wall: text around it goes, the image stays, and the two
    /// sides never disagree about how many images exist.
    fn attachment_floor(&self, cursor: usize) -> usize {
        (0..cursor)
            .rev()
            .find(|index| self.is_opaque_attachment(*index))
            .map_or(0, |index| index + 1)
    }

    fn attachment_ceiling(&self, cursor: usize) -> usize {
        (cursor..self.atoms.len())
            .find(|index| self.is_opaque_attachment(*index))
            .unwrap_or(self.atoms.len())
    }

    fn is_opaque_attachment(&self, index: usize) -> bool {
        matches!(self.atoms.get(index), Some(EditorAtom::Attachment(_)))
    }

    /// Whether an atom stands for something the composer is not spelling out —
    /// a collapsed paste or an image. Either is one indivisible word.
    fn is_opaque(&self, index: usize) -> bool {
        matches!(
            self.atoms.get(index),
            Some(EditorAtom::LargePaste { .. } | EditorAtom::Attachment(_))
        )
    }

    fn rebuild_projections(&mut self) {
        self.source_text.clear();
        self.display_text.clear();
        self.source_boundaries.clear();
        self.display_boundaries.clear();
        self.source_boundaries.push(0);
        self.display_boundaries.push(0);

        let mut attachment_index = 0;
        for atom in &self.atoms {
            match atom {
                EditorAtom::Character(character) => {
                    self.source_text.push(*character);
                    self.display_text.push(*character);
                }
                EditorAtom::LargePaste {
                    source_text,
                    character_count,
                } => {
                    self.source_text.push_str(source_text);
                    self.display_text
                        .push_str(&large_paste_marker(*character_count));
                }
                EditorAtom::Attachment(attachment) => {
                    attachment_index += 1;
                    self.display_text
                        .push_str(&attachment_marker(&attachment.display, attachment_index));
                }
                EditorAtom::Gap => self.display_text.push(' '),
            }
            self.source_boundaries.push(self.source_text.len());
            self.display_boundaries.push(self.display_text.len());
        }

        debug_assert!(self.cursor <= self.atoms.len());
    }

    fn paste_ranges(&self) -> Vec<PasteRange> {
        self.atoms
            .iter()
            .zip(self.source_boundaries.windows(2))
            .filter_map(|(atom, range)| match atom {
                EditorAtom::LargePaste {
                    character_count, ..
                } => Some(PasteRange {
                    start_byte: range[0],
                    end_byte: range[1],
                    character_count: *character_count,
                }),
                EditorAtom::Character(_) | EditorAtom::Attachment(_) | EditorAtom::Gap => None,
            })
            .collect()
    }

    fn move_vertical(&mut self, direction: i8) {
        let display_cursor = self.display_cursor_byte();
        let line_start = self.display_text[..display_cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let column = self.preferred_column.unwrap_or_else(|| {
            self.display_text[line_start..display_cursor]
                .chars()
                .map(|character| character.width().unwrap_or(0))
                .sum()
        });
        let target_start = if direction < 0 {
            if line_start == 0 {
                return;
            }
            self.display_text[..line_start - 1]
                .rfind('\n')
                .map(|index| index + 1)
                .unwrap_or(0)
        } else {
            let line_end = self.display_text[display_cursor..]
                .find('\n')
                .map(|index| display_cursor + index);
            let Some(line_end) = line_end else {
                return;
            };
            line_end + 1
        };
        let target_end = self.display_text[target_start..]
            .find('\n')
            .map(|index| target_start + index)
            .unwrap_or(self.display_text.len());
        let target = byte_at_display_column(&self.display_text, target_start, target_end, column);
        self.cursor = self.closest_display_boundary(target);
        self.preferred_column = Some(column);
    }

    fn closest_display_boundary(&self, target: usize) -> usize {
        match self.display_boundaries.binary_search(&target) {
            Ok(index) => index,
            Err(next) => {
                if next == 0 {
                    return 0;
                }
                if next == self.display_boundaries.len() {
                    return self.display_boundaries.len() - 1;
                }

                let previous = next - 1;
                let previous_distance = target - self.display_boundaries[previous];
                let next_distance = self.display_boundaries[next] - target;
                if previous_distance <= next_distance {
                    previous
                } else {
                    next
                }
            }
        }
    }
}

pub fn staged_image_path(text: &str) -> Option<PathBuf> {
    let path = PathBuf::from(text.trim());
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let image_extension = matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    );
    let staged = path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .starts_with("herdr-clipboard-images-")
    });
    (image_extension && staged && path.is_file()).then_some(path)
}

/// Agents number an image when it is pasted and keep that number for the rest
/// of the session, so the label the pane gave it is shown rather than its place
/// in the line. Only an image with no label of its own falls back to counting.
fn attachment_marker(label: &str, index: usize) -> String {
    if label.starts_with("Image #") {
        format!("[{label}]")
    } else {
        format!("[Image #{index}]")
    }
}

fn byte_at_display_column(text: &str, start: usize, end: usize, column: usize) -> usize {
    let mut width = 0;
    for (offset, character) in text[start..end].char_indices() {
        let next = width + character.width().unwrap_or(0);
        if next > column {
            return start + offset;
        }
        width = next;
    }
    end
}
