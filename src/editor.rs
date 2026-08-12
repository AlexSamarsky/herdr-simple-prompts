use std::path::PathBuf;

use unicode_width::UnicodeWidthChar;

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
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EditorSnapshot {
    pub chunks: Vec<EditorChunk>,
}

impl EditorSnapshot {
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
            })
            .sum();
        let mut submission = String::with_capacity(capacity);
        for chunk in &self.chunks {
            match chunk {
                EditorChunk::Text(text) => submission.push_str(text),
                EditorChunk::LargePaste { source_text, .. } => submission.push_str(source_text),
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
    LargePaste {
        source_text: String,
        character_count: usize,
    },
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

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.atoms.remove(self.cursor);
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn delete(&mut self) {
        if self.cursor == self.atoms.len() {
            return;
        }
        self.atoms.remove(self.cursor);
        self.preferred_column = None;
        self.rebuild_projections();
    }

    pub fn move_home(&mut self) {
        let display_cursor = self.display_cursor_byte();
        let target = self.display_text[..display_cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        self.cursor = self.closest_display_boundary(target);
        self.preferred_column = None;
    }

    pub fn move_end(&mut self) {
        let display_cursor = self.display_cursor_byte();
        let target = self.display_text[display_cursor..]
            .find('\n')
            .map(|index| display_cursor + index)
            .unwrap_or(self.display_text.len());
        self.cursor = self.closest_display_boundary(target);
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

    fn rebuild_projections(&mut self) {
        self.source_text.clear();
        self.display_text.clear();
        self.source_boundaries.clear();
        self.display_boundaries.clear();
        self.source_boundaries.push(0);
        self.display_boundaries.push(0);

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
                EditorAtom::Character(_) => None,
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
