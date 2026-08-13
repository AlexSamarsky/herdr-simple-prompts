use crate::app::AppState;
use crate::local_time::{format_local_timestamp, format_timestamp_at_offset};
use crate::model::{Delivery, Message};
use crate::style::{AnsiColor, MessagePresentation, StyleModifiers, StyleRun, StyledText};
use chrono::FixedOffset;
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CellStyle {
    pub foreground: Option<AnsiColor>,
    pub background: Option<AnsiColor>,
    pub modifiers: StyleModifiers,
}

impl From<&StyleRun> for CellStyle {
    fn from(run: &StyleRun) -> Self {
        Self {
            foreground: run.foreground,
            background: run.background,
            modifiers: run.modifiers,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualSpan {
    pub text: String,
    pub style: CellStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub spans: Vec<VisualSpan>,
    pub fill: Option<CellStyle>,
}

impl VisualRow {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            spans: vec![VisualSpan {
                text: text.into(),
                style: CellStyle::default(),
            }],
            fill: None,
        }
    }

    pub fn cell_width(&self) -> usize {
        self.spans
            .iter()
            .flat_map(|span| span.text.chars())
            .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
            .sum()
    }

    pub fn plain_text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    fn push_char(&mut self, character: char, style: CellStyle) {
        if let Some(previous) = self.spans.last_mut()
            && previous.style == style
        {
            previous.text.push(character);
        } else {
            let mut text = String::with_capacity(character.len_utf8());
            text.push(character);
            self.spans.push(VisualSpan { text, style });
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptSection {
    pub start_row: usize,
    pub content_start_row: usize,
    pub prompt_rows: usize,
    pub end_row: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryDocument {
    pub rows: Vec<VisualRow>,
    pub prompts: Vec<PromptSection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StickyRows {
    pub source_start: usize,
    pub screen_start: usize,
    pub count: usize,
}

impl HistoryDocument {
    pub fn from_app(app: &AppState, width: u16) -> Self {
        Self::from_app_with_timestamp_formatter(app, width, format_local_timestamp)
    }

    pub fn from_app_at_offset(app: &AppState, width: u16, offset: FixedOffset) -> Self {
        Self::from_app_with_timestamp_formatter(app, width, |timestamp_ms| {
            format_timestamp_at_offset(timestamp_ms, offset)
        })
    }

    fn from_app_with_timestamp_formatter(
        app: &AppState,
        width: u16,
        mut format_timestamp: impl FnMut(Option<u64>) -> Option<String>,
    ) -> Self {
        let mut document = Self::default();
        let width = usize::from(width.max(1));
        for turn in &app.turns {
            let start_row = document.rows.len();
            document.rows.push(filled_timestamp_row(
                format_timestamp(turn.prompt.timestamp_ms).as_deref(),
                prompt_fill(),
                width,
            ));
            let content_start_row = document.rows.len();
            let mut prompt_lines = prompt_lines(&turn.prompt, &turn.delivery);
            if prompt_lines.is_empty() {
                prompt_lines.push(StyledText::default());
            }
            for line in &prompt_lines {
                push_styled_rows(&mut document.rows, line, prompt_fill(), width);
            }
            let prompt_rows = document.rows.len() - content_start_row;
            document.rows.push(filled_empty_row(prompt_fill()));

            if let Some(answer) = &turn.final_answer {
                if let Some(timestamp) = format_timestamp(answer.timestamp_ms) {
                    document
                        .rows
                        .push(filled_timestamp_row(Some(timestamp.as_str()), None, width));
                }
                for line in &answer_lines(answer) {
                    push_answer_rows(&mut document.rows, line, width);
                }
            }
            document.rows.push(empty_row());
            document.prompts.push(PromptSection {
                start_row,
                content_start_row,
                prompt_rows,
                end_row: document.rows.len(),
            });
        }
        document
    }

    pub fn viewport(&self, height: usize, scroll_from_bottom: usize) -> Vec<VisualRow> {
        let visible_height = height.min(self.rows.len());
        if visible_height == 0 {
            return Vec::new();
        }
        let maximum_offset = self.rows.len().saturating_sub(visible_height);
        let offset = scroll_from_bottom.min(maximum_offset);
        let top = maximum_offset.saturating_sub(offset);
        let sticky = sticky_overlay(&self.prompts, top, visible_height);
        let mut visible = Vec::with_capacity(visible_height);
        if let Some(sticky) = sticky {
            visible.extend(
                self.rows[sticky.source_start..sticky.source_start + sticky.count]
                    .iter()
                    .cloned(),
            );
            visible.extend(
                self.rows[top + sticky.count..top + visible_height]
                    .iter()
                    .cloned(),
            );
        } else {
            visible.extend(self.rows[top..top + visible_height].iter().cloned());
        }
        visible
    }
}

pub fn sticky_overlay(sections: &[PromptSection], top: usize, height: usize) -> Option<StickyRows> {
    let section_index = sections
        .iter()
        .rposition(|section| section.content_start_row < top && top < section.end_row)?;
    let section = sections[section_index];
    let content_count = 2.min(section.prompt_rows).min(height.saturating_sub(1));
    if content_count == 0 {
        return None;
    }
    let include_padding = height >= content_count + 2;
    let desired_count = content_count + usize::from(include_padding);
    let base_source_start = if include_padding {
        section.start_row
    } else {
        section.content_start_row
    };
    let mut count = desired_count;
    if let Some(next) = sections.get(section_index + 1) {
        count = count.min(next.start_row.saturating_sub(top));
    }
    (count > 0).then_some(StickyRows {
        source_start: base_source_start + (desired_count - count),
        screen_start: 0,
        count,
    })
}

pub fn wrap_styled(source: &StyledText, width: usize) -> Vec<VisualRow> {
    let width = width.max(1);
    let runs = valid_runs(source);
    let mut rows = Vec::new();
    let mut styles = StyleCursor::new(&runs);
    let mut row = empty_row();
    let mut row_width = 0;
    let mut token_start = 0;
    while token_start < source.text.len() {
        let rest = &source.text[token_start..];
        let token_end = rest
            .char_indices()
            .find_map(|(index, character)| (character == '\n').then_some(token_start + index))
            .unwrap_or(source.text.len());
        let line = &source.text[token_start..token_end];
        for (offset, word) in WordBoundaries::new(line) {
            let word_width = word
                .chars()
                .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
                .sum::<usize>();
            if word_width <= width && row_width > 0 && row_width + word_width > width {
                rows.push(row);
                row = empty_row();
                row_width = 0;
            }
            for (word_offset, character) in word.char_indices() {
                let byte = token_start + offset + word_offset;
                let style = styles.style_at(byte);
                let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
                if character_width > 0 && row_width > 0 && row_width + character_width > width {
                    rows.push(row);
                    row = empty_row();
                    row_width = 0;
                }
                row.push_char(character, style);
                row_width += character_width;
            }
        }
        if token_end == source.text.len() {
            break;
        }
        rows.push(row);
        row = empty_row();
        row_width = 0;
        token_start = token_end + 1;
    }
    rows.push(row);
    rows
}

struct WordBoundaries<'a> {
    text: &'a str,
    offset: usize,
}

impl<'a> WordBoundaries<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, offset: 0 }
    }
}

impl<'a> Iterator for WordBoundaries<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.text.len() {
            return None;
        }
        let start = self.offset;
        let whitespace = self.text[start..].chars().next()?.is_whitespace();
        let mut end = self.text.len();
        for (offset, character) in self.text[start..].char_indices().skip(1) {
            if character.is_whitespace() != whitespace {
                end = start + offset;
                break;
            }
        }
        self.offset = end;
        Some((start, &self.text[start..end]))
    }
}

struct StyleCursor<'a> {
    runs: &'a [StyleRun],
    index: usize,
}

impl<'a> StyleCursor<'a> {
    fn new(runs: &'a [StyleRun]) -> Self {
        Self { runs, index: 0 }
    }

    fn style_at(&mut self, byte: usize) -> CellStyle {
        while self
            .runs
            .get(self.index)
            .is_some_and(|run| byte >= run.end_byte)
        {
            self.index += 1;
        }
        self.runs
            .get(self.index)
            .filter(|run| run.start_byte <= byte)
            .map(CellStyle::from)
            .unwrap_or_default()
    }
}

fn valid_runs(source: &StyledText) -> Vec<StyleRun> {
    let mut previous_end = 0;
    source
        .runs
        .iter()
        .filter(|run| {
            let valid = run.start_byte < run.end_byte
                && run.end_byte <= source.text.len()
                && source.text.is_char_boundary(run.start_byte)
                && source.text.is_char_boundary(run.end_byte)
                && run.start_byte >= previous_end;
            if valid {
                previous_end = run.end_byte;
            }
            valid
        })
        .cloned()
        .collect()
}

fn prompt_lines(message: &Message, delivery: &Delivery) -> Vec<StyledText> {
    let mut lines = message
        .text
        .split('\n')
        .map(|text| StyledText {
            text: text.into(),
            runs: Vec::new(),
        })
        .collect::<Vec<_>>();
    if message.text.is_empty()
        && let Some(attachment) = message.attachments.first()
    {
        lines[0].text = format!("[Image #1] {}", attachment.display);
    }
    let skip = if message.text.is_empty() && !message.attachments.is_empty() {
        1
    } else {
        0
    };
    lines.extend(
        message
            .attachments
            .iter()
            .enumerate()
            .skip(skip)
            .map(|(index, attachment)| StyledText {
                text: format!("[Image #{}] {}", index + 1, attachment.display),
                runs: Vec::new(),
            }),
    );
    if let Delivery::Failed { reason } = delivery {
        lines.push(StyledText {
            text: format!("not sent: {reason}"),
            runs: Vec::new(),
        });
    }
    lines
}

fn answer_lines(message: &Message) -> Vec<StyledText> {
    let source = match &message.presentation {
        MessagePresentation::NativeAnsi(styled) => styled.clone(),
        MessagePresentation::MarkdownFallback => crate::markdown::style_markdown(&message.text),
        MessagePresentation::Plain => StyledText {
            text: message.text.clone(),
            runs: Vec::new(),
        },
    };
    split_styled_lines(&source)
}

fn split_styled_lines(source: &StyledText) -> Vec<StyledText> {
    let mut lines = Vec::new();
    let mut start = 0;
    for end in source
        .text
        .match_indices('\n')
        .map(|(index, _)| index)
        .chain(std::iter::once(source.text.len()))
    {
        let text = source.text[start..end].to_owned();
        let runs = source
            .runs
            .iter()
            .filter_map(|run| {
                let from = run.start_byte.max(start);
                let to = run.end_byte.min(end);
                (from < to).then(|| StyleRun {
                    start_byte: from - start,
                    end_byte: to - start,
                    foreground: run.foreground,
                    background: run.background,
                    modifiers: run.modifiers,
                })
            })
            .collect();
        lines.push(StyledText { text, runs });
        start = end.saturating_add(1);
    }
    lines
}

fn push_styled_rows(
    rows: &mut Vec<VisualRow>,
    source: &StyledText,
    fill: Option<CellStyle>,
    width: usize,
) {
    for mut row in wrap_styled(source, width) {
        row.fill = fill;
        rows.push(row);
    }
}

fn push_answer_rows(rows: &mut Vec<VisualRow>, source: &StyledText, width: usize) {
    for mut row in wrap_styled(source, width) {
        for span in &mut row.spans {
            if span.style.foreground.is_none() {
                span.style.foreground = Some(AnsiColor::BrightWhite);
            }
        }
        rows.push(row);
    }
}

fn prompt_fill() -> Option<CellStyle> {
    Some(CellStyle {
        foreground: Some(AnsiColor::BrightWhite),
        background: Some(AnsiColor::Rgb(52, 53, 54)),
        modifiers: StyleModifiers::default(),
    })
}

fn empty_row() -> VisualRow {
    VisualRow {
        spans: Vec::new(),
        fill: None,
    }
}

fn filled_empty_row(fill: Option<CellStyle>) -> VisualRow {
    VisualRow {
        spans: Vec::new(),
        fill,
    }
}

fn filled_timestamp_row(
    timestamp: Option<&str>,
    fill: Option<CellStyle>,
    width: usize,
) -> VisualRow {
    let clipped = timestamp.map(|timestamp| clip_to_width(timestamp, width));
    let spans = clipped
        .filter(|timestamp| !timestamp.is_empty())
        .map(|timestamp| {
            vec![VisualSpan {
                text: timestamp,
                style: CellStyle {
                    foreground: Some(AnsiColor::White),
                    ..CellStyle::default()
                },
            }]
        })
        .unwrap_or_default();
    VisualRow { spans, fill }
}

fn clip_to_width(text: &str, width: usize) -> String {
    let mut clipped = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if character_width > 0 && used + character_width > width {
            break;
        }
        clipped.push(character);
        used += character_width;
    }
    clipped
}
