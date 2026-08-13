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

#[derive(Clone, Copy, Debug)]
struct LineRange {
    start: usize,
    end: usize,
    after_end: usize,
}

pub fn style_markdown(text: &str) -> StyledText {
    let mut slots = vec![StyleSlot::default(); text.len()];
    let mut visible = vec![true; text.len()];
    let lines = line_ranges(text);
    let mut line_index = 0;

    while line_index < lines.len() {
        let line_range = lines[line_index];
        let line = &text[line_range.start..line_range.end];
        if is_opening_fence(line) {
            let Some(closing_index) =
                lines
                    .iter()
                    .enumerate()
                    .skip(line_index + 1)
                    .find_map(|(index, candidate)| {
                        is_closing_fence(&text[candidate.start..candidate.end]).then_some(index)
                    })
            else {
                break;
            };

            discard(&mut visible, line_range.start, line_range.after_end);
            for code_line in &lines[line_index + 1..closing_index] {
                apply_style(
                    &mut slots,
                    code_line.start,
                    code_line.end,
                    FENCED_CODE_PRIORITY,
                    code_style(),
                );
            }
            let closing_range = lines[closing_index];
            discard(&mut visible, closing_range.start, closing_range.after_end);
            line_index = closing_index + 1;
            continue;
        }

        let (content_offset, heading) = block_content(line);
        let content_start = line_range.start + content_offset;
        if heading {
            discard(&mut visible, line_range.start, content_start);
            apply_style(
                &mut slots,
                content_start,
                line_range.end,
                BLOCK_PRIORITY,
                bold_style(),
            );
        }
        style_inline(
            text,
            content_start,
            line_range.end,
            &mut slots,
            &mut visible,
        );
        line_index += 1;
    }

    project_visible(text, &slots, &visible)
}

fn line_ranges(text: &str) -> Vec<LineRange> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (end, _) in text.match_indices('\n') {
        ranges.push(LineRange {
            start,
            end,
            after_end: end + 1,
        });
        start = end + 1;
    }
    if start < text.len() || text.is_empty() {
        ranges.push(LineRange {
            start,
            end: text.len(),
            after_end: text.len(),
        });
    }
    ranges
}

fn project_visible(text: &str, slots: &[StyleSlot], visible: &[bool]) -> StyledText {
    let mut projected = String::with_capacity(text.len());
    let mut builder = StyleRunBuilder::new();
    for (byte, character) in text.char_indices() {
        if visible[byte] {
            builder.set_style(slots[byte].style, projected.len());
            projected.push(character);
        }
    }
    let runs = builder.finish(projected.len());
    StyledText {
        text: projected,
        runs,
    }
}

fn style_inline(
    text: &str,
    start: usize,
    end: usize,
    slots: &mut [StyleSlot],
    visible: &mut [bool],
) {
    if start >= end {
        return;
    }
    style_inline_code(text, start, end, slots, visible);
    style_links(text, start, end, slots, visible);
    style_strong(text, start, end, slots, visible);
    style_emphasis(text, start, end, slots, visible);
}

fn style_inline_code(
    text: &str,
    start: usize,
    end: usize,
    slots: &mut [StyleSlot],
    visible: &mut [bool],
) {
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
        discard_if_allowed(
            visible,
            slots,
            &[(open, content_start), (close, close + 1)],
            INLINE_CODE_PRIORITY,
        );
        cursor = close + 1;
    }
}

fn style_links(
    text: &str,
    start: usize,
    end: usize,
    slots: &mut [StyleSlot],
    visible: &mut [bool],
) {
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
            discard_if_allowed(
                visible,
                slots,
                &[(open, label_start), (label_end, close + 1)],
                LINK_LABEL_PRIORITY,
            );
        }
        cursor = close + 1;
    }
}

fn style_strong(
    text: &str,
    start: usize,
    end: usize,
    slots: &mut [StyleSlot],
    visible: &mut [bool],
) {
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
        discard_if_allowed(
            visible,
            slots,
            &[(open, content_start), (close, close + 2)],
            STRONG_PRIORITY,
        );
        cursor = close + 2;
    }
}

fn style_emphasis(
    text: &str,
    start: usize,
    end: usize,
    slots: &mut [StyleSlot],
    visible: &mut [bool],
) {
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
        discard_if_allowed(
            visible,
            slots,
            &[(open, content_start), (close, close + 1)],
            EMPHASIS_PRIORITY,
        );
        cursor = close + 1;
    }
}

fn discard_if_allowed(
    visible: &mut [bool],
    slots: &[StyleSlot],
    ranges: &[(usize, usize)],
    priority: u8,
) {
    if ranges.iter().all(|&(start, end)| {
        slots[start..end]
            .iter()
            .all(|slot| slot.priority <= priority)
    }) {
        for &(start, end) in ranges {
            discard(visible, start, end);
        }
    }
}

fn discard(visible: &mut [bool], start: usize, end: usize) {
    visible[start..end].fill(false);
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
