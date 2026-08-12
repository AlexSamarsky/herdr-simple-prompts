use crate::style::{AnsiColor, StyleModifiers, StyleRunBuilder, StyleState, StyledText};

const BLOCK_PRIORITY: u8 = 1;
const EMPHASIS_PRIORITY: u8 = 2;
const STRONG_PRIORITY: u8 = 3;
const LINK_PRIORITY: u8 = 4;
const LINK_LABEL_PRIORITY: u8 = 5;
const INLINE_CODE_PRIORITY: u8 = 6;
const FENCED_CODE_PRIORITY: u8 = 7;

#[derive(Clone, Copy, Debug, Default)]
struct StyleSlot {
    priority: u8,
    style: StyleState,
}

pub fn style_markdown(text: &str) -> StyledText {
    let mut slots = vec![StyleSlot::default(); text.len()];
    let mut fenced = false;
    let mut line_start = 0;

    for line_end in text
        .match_indices('\n')
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
    {
        let line = &text[line_start..line_end];
        if fenced {
            if is_closing_fence(line) {
                fenced = false;
            } else {
                apply_style(
                    &mut slots,
                    line_start,
                    line_end,
                    FENCED_CODE_PRIORITY,
                    code_style(),
                );
            }
        } else if is_opening_fence(line) {
            fenced = true;
        } else {
            let (content_offset, heading) = block_content(line);
            let content_start = line_start + content_offset;
            if heading {
                apply_style(
                    &mut slots,
                    content_start,
                    line_end,
                    BLOCK_PRIORITY,
                    bold_style(),
                );
            }
            style_inline(text, content_start, line_end, &mut slots);
        }
        line_start = line_end.saturating_add(1);
    }

    let mut builder = StyleRunBuilder::new();
    for (byte, _) in text.char_indices() {
        builder.set_style(slots[byte].style, byte);
    }
    let runs = builder.finish(text.len());
    StyledText {
        text: text.to_owned(),
        runs,
    }
}

fn style_inline(text: &str, start: usize, end: usize, slots: &mut [StyleSlot]) {
    if start >= end {
        return;
    }
    style_emphasis(text, start, end, slots);
    style_strong(text, start, end, slots);
    style_links(text, start, end, slots);
    style_inline_code(text, start, end, slots);
}

fn style_inline_code(text: &str, start: usize, end: usize, slots: &mut [StyleSlot]) {
    let bytes = text.as_bytes();
    let mut cursor = start;
    while let Some(open) = find_byte(bytes, cursor, end, b'`') {
        let content_start = open + 1;
        let Some(close) = find_byte(bytes, content_start, end, b'`') else {
            break;
        };
        if content_start < close {
            apply_style(
                slots,
                content_start,
                close,
                INLINE_CODE_PRIORITY,
                code_style(),
            );
        }
        cursor = close + 1;
    }
}

fn style_links(text: &str, start: usize, end: usize, slots: &mut [StyleSlot]) {
    let bytes = text.as_bytes();
    let mut cursor = start;
    while let Some(open) = find_byte(bytes, cursor, end, b'[') {
        let label_start = open + 1;
        let Some(label_end) = find_sequence(bytes, label_start, end, b"](") else {
            break;
        };
        if let Some(nested_open) = find_byte(bytes, label_start, label_end, b'[') {
            cursor = nested_open;
            continue;
        }
        let url_start = label_end + 2;
        let Some(close) = find_byte(bytes, url_start, end, b')') else {
            break;
        };
        if let Some(nested_open) = find_byte(bytes, url_start, close, b'[') {
            cursor = nested_open;
            continue;
        }
        let url = &text[url_start..close];
        if label_start < label_end && !url.is_empty() && !url.chars().any(char::is_whitespace) {
            apply_style(slots, open, close + 1, LINK_PRIORITY, StyleState::default());
            apply_style(
                slots,
                label_start,
                label_end,
                LINK_LABEL_PRIORITY,
                link_style(),
            );
        }
        cursor = close + 1;
    }
}

fn style_strong(text: &str, start: usize, end: usize, slots: &mut [StyleSlot]) {
    let bytes = text.as_bytes();
    let mut cursor = start;
    while let Some(open) = find_sequence(bytes, cursor, end, b"**") {
        let content_start = open + 2;
        let Some(close) = find_sequence(bytes, content_start, end, b"**") else {
            break;
        };
        if content_start < close {
            apply_style(slots, content_start, close, STRONG_PRIORITY, bold_style());
        }
        cursor = close + 2;
    }
}

fn style_emphasis(text: &str, start: usize, end: usize, slots: &mut [StyleSlot]) {
    let bytes = text.as_bytes();
    let mut cursor = start;
    while let Some(open) = find_byte(bytes, cursor, end, b'_') {
        let content_start = open + 1;
        let Some(close) = find_byte(bytes, content_start, end, b'_') else {
            break;
        };
        if content_start < close {
            apply_style(
                slots,
                content_start,
                close,
                EMPHASIS_PRIORITY,
                italic_style(),
            );
        }
        cursor = close + 1;
    }
}

fn apply_style(slots: &mut [StyleSlot], start: usize, end: usize, priority: u8, style: StyleState) {
    for slot in &mut slots[start..end] {
        if priority > slot.priority {
            *slot = StyleSlot { priority, style };
        }
    }
}

fn block_content(line: &str) -> (usize, bool) {
    if let Some(content) = line.strip_prefix("## ") {
        return (line.len() - content.len(), true);
    }
    if let Some(content) = line.strip_prefix("# ") {
        return (line.len() - content.len(), true);
    }
    if let Some(content) = line.strip_prefix("- ") {
        return (line.len() - content.len(), false);
    }
    let digit_count = line.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count > 0 && line[digit_count..].starts_with(". ") {
        return (digit_count + 2, false);
    }
    (0, false)
}

fn is_opening_fence(line: &str) -> bool {
    fence_body(line).is_some()
}

fn is_closing_fence(line: &str) -> bool {
    fence_body(line).is_some_and(|suffix| suffix.trim().is_empty())
}

fn fence_body(line: &str) -> Option<&str> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let suffix = line[indent..].strip_prefix("```")?;
    (!suffix.starts_with('`')).then_some(suffix)
}

fn find_byte(bytes: &[u8], start: usize, end: usize, needle: u8) -> Option<usize> {
    bytes[start..end]
        .iter()
        .position(|byte| *byte == needle)
        .map(|offset| start + offset)
}

fn find_sequence(bytes: &[u8], start: usize, end: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || end.saturating_sub(start) < needle.len() {
        return None;
    }
    bytes[start..end]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| start + offset)
}

fn bold_style() -> StyleState {
    StyleState {
        modifiers: StyleModifiers {
            bold: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn italic_style() -> StyleState {
    StyleState {
        modifiers: StyleModifiers {
            italic: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn link_style() -> StyleState {
    StyleState {
        foreground: Some(AnsiColor::Cyan),
        modifiers: StyleModifiers {
            underline: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn code_style() -> StyleState {
    StyleState {
        foreground: Some(AnsiColor::White),
        background: Some(AnsiColor::BrightBlack),
        modifiers: StyleModifiers::default(),
    }
}
