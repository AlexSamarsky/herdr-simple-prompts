use std::path::PathBuf;
use unicode_width::UnicodeWidthChar;

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

#[derive(Clone, Default)]
pub struct Editor {
    text: String,
    cursor: usize,
    preferred_column: Option<usize>,
}

impl Editor {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor_byte(&self) -> usize {
        self.cursor
    }

    pub fn insert_char(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
        self.preferred_column = None;
    }

    pub fn insert_paste(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.preferred_column = None;
    }

    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = previous_boundary(&self.text, self.cursor);
        self.preferred_column = None;
    }

    pub fn move_right(&mut self) {
        if self.cursor == self.text.len() {
            return;
        }
        self.cursor = next_boundary(&self.text, self.cursor);
        self.preferred_column = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = previous_boundary(&self.text, self.cursor);
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
        self.preferred_column = None;
    }

    pub fn delete(&mut self) {
        if self.cursor == self.text.len() {
            return;
        }
        let next = next_boundary(&self.text, self.cursor);
        self.text.drain(self.cursor..next);
        self.preferred_column = None;
    }

    pub fn move_home(&mut self) {
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        self.preferred_column = None;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.text.len());
        self.preferred_column = None;
    }

    pub fn move_up(&mut self) {
        self.move_vertical(-1);
    }

    pub fn move_down(&mut self) {
        self.move_vertical(1);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_column = None;
    }

    pub fn replace(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.preferred_column = None;
    }

    pub fn take_submission(&mut self) -> String {
        self.cursor = 0;
        self.preferred_column = None;
        std::mem::take(&mut self.text)
    }

    fn move_vertical(&mut self, direction: i8) {
        let line_start = self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let column = self.preferred_column.unwrap_or_else(|| {
            self.text[line_start..self.cursor]
                .chars()
                .map(|character| character.width().unwrap_or(0))
                .sum()
        });
        let target_start = if direction < 0 {
            if line_start == 0 {
                return;
            }
            self.text[..line_start - 1]
                .rfind('\n')
                .map(|index| index + 1)
                .unwrap_or(0)
        } else {
            let line_end = self.text[self.cursor..]
                .find('\n')
                .map(|index| self.cursor + index);
            let Some(line_end) = line_end else {
                return;
            };
            line_end + 1
        };
        let target_end = self.text[target_start..]
            .find('\n')
            .map(|index| target_start + index)
            .unwrap_or(self.text.len());
        self.cursor = byte_at_display_column(&self.text, target_start, target_end, column);
        self.preferred_column = Some(column);
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

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .char_indices()
        .nth(1)
        .map(|(index, _)| cursor + index)
        .unwrap_or(text.len())
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
