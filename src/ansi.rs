use crate::agent::AgentKind;
use crate::style::{AnsiColor, StyleRun, StyleRunBuilder, StyleState, StyledText};

struct NativeChrome {
    role_prefixes: &'static [&'static str],
    continuation_prefixes: &'static [&'static str],
    separator_min_width: usize,
    trailing_boundary_prefixes: &'static [&'static str],
    composer_prefixes: &'static [&'static str],
    footer_prefixes: &'static [&'static str],
}

const CODEX_CHROME: NativeChrome = NativeChrome {
    role_prefixes: &["• "],
    continuation_prefixes: &["  "],
    separator_min_width: 8,
    trailing_boundary_prefixes: &["─ Worked for "],
    composer_prefixes: &["› "],
    footer_prefixes: &["gpt-"],
};

const CLAUDE_CHROME: NativeChrome = NativeChrome {
    role_prefixes: &["⏺ "],
    continuation_prefixes: &["  "],
    separator_min_width: 16,
    trailing_boundary_prefixes: &[],
    composer_prefixes: &["❯ "],
    footer_prefixes: &["Claude", "Opus"],
};

pub fn sanitize_ansi(input: &str) -> StyledText {
    let bytes = input.as_bytes();
    let mut text = String::with_capacity(input.len());
    let mut runs = StyleRunBuilder::new();
    let mut style = StyleState::default();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\x1b' => index = consume_escape(bytes, index, &mut style, &mut runs, text.len()),
            b'\r' => {
                text.push('\n');
                index += usize::from(bytes.get(index + 1) == Some(&b'\n')) + 1;
            }
            b'\n' => {
                text.push('\n');
                index += 1;
            }
            0x00..=0x1f | 0x7f => index += 1,
            _ => {
                let character = input[index..]
                    .chars()
                    .next()
                    .expect("index is always on a UTF-8 boundary");
                if !character.is_control() {
                    text.push(character);
                }
                index += character.len_utf8();
            }
        }
    }

    StyledText {
        runs: runs.finish(text.len()),
        text,
    }
}

pub fn extract_native_final(ansi: &str, canonical: &str, kind: AgentKind) -> Option<StyledText> {
    if canonical.is_empty() || canonical.contains('\r') {
        return None;
    }
    let sanitized = sanitize_ansi(ansi);
    let lines = line_ranges(&sanitized.text);
    let chrome = match kind {
        AgentKind::Codex => &CODEX_CHROME,
        AgentKind::Claude => &CLAUDE_CHROME,
    };
    let canonical_lines: Vec<&str> = canonical.split('\n').collect();
    let mut candidates = Vec::new();

    for boundary in 0..lines.len() {
        if !is_pure_separator(
            line_text(&sanitized.text, lines[boundary]),
            chrome.separator_min_width,
        ) {
            continue;
        }
        let first = boundary + 1;
        let trailing = first + canonical_lines.len();
        let composer = trailing + 1;
        if composer >= lines.len()
            || !is_trailing_boundary(line_text(&sanitized.text, lines[trailing]), chrome)
            || !starts_with_any(
                line_text(&sanitized.text, lines[composer]),
                chrome.composer_prefixes,
            )
        {
            continue;
        }

        let mut mappings = Vec::with_capacity(canonical_lines.len() * 2);
        let mut destination = 0;
        let mut exact = true;
        for (offset, canonical_line) in canonical_lines.iter().enumerate() {
            let range = lines[first + offset];
            let source_line = line_text(&sanitized.text, range);
            let prefixes = if offset == 0 {
                chrome.role_prefixes
            } else {
                chrome.continuation_prefixes
            };
            let prefix = if source_line.is_empty() && canonical_line.is_empty() {
                ""
            } else if let Some(prefix) = prefixes
                .iter()
                .copied()
                .find(|prefix| source_line.starts_with(prefix))
            {
                prefix
            } else {
                exact = false;
                break;
            };
            let content_start = range.start + prefix.len();
            if &sanitized.text[content_start..range.end] != *canonical_line {
                exact = false;
                break;
            }
            mappings.push((content_start, range.end, destination));
            destination += canonical_line.len();
            if offset + 1 < canonical_lines.len() {
                let newline_end = range.end.checked_add(1)?;
                if sanitized.text.as_bytes().get(range.end) != Some(&b'\n') {
                    exact = false;
                    break;
                }
                mappings.push((range.end, newline_end, destination));
                destination += 1;
            }
        }
        if exact {
            candidates.push((slice_mapped_runs(&sanitized.runs, &mappings), composer));
        }
    }

    if candidates.len() != 1 {
        return None;
    }
    let (runs, composer) = candidates.pop().expect("one candidate");
    if lines[composer + 1..]
        .iter()
        .map(|range| line_text(&sanitized.text, *range))
        .filter(|line| !line.is_empty())
        .any(|line| !starts_with_any(line, chrome.footer_prefixes))
    {
        return None;
    }
    Some(StyledText {
        text: canonical.to_owned(),
        runs,
    })
}

#[derive(Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

fn line_ranges(text: &str) -> Vec<LineRange> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push(LineRange { start, end: index });
            start = index + 1;
        }
    }
    if start < text.len() || text.ends_with('\n') {
        ranges.push(LineRange {
            start,
            end: text.len(),
        });
    }
    ranges
}

fn line_text(text: &str, range: LineRange) -> &str {
    &text[range.start..range.end]
}

fn is_pure_separator(line: &str, minimum_width: usize) -> bool {
    let width = line.chars().count();
    width >= minimum_width && line.chars().all(|character| character == '─')
}

fn is_trailing_boundary(line: &str, chrome: &NativeChrome) -> bool {
    is_pure_separator(line, chrome.separator_min_width)
        || chrome
            .trailing_boundary_prefixes
            .iter()
            .any(|prefix| matches_decorated_boundary(line, prefix, chrome.separator_min_width))
}

fn matches_decorated_boundary(line: &str, prefix: &str, minimum_width: usize) -> bool {
    if line.chars().count() < minimum_width || !line.starts_with(prefix) {
        return false;
    }
    let remainder = &line[prefix.len()..];
    let Some((label, suffix)) = remainder.rsplit_once(" ") else {
        return false;
    };
    valid_elapsed_label(label)
        && !suffix.is_empty()
        && suffix.chars().all(|character| character == '─')
}

fn valid_elapsed_label(label: &str) -> bool {
    let mut parts = label.split_ascii_whitespace().peekable();
    if parts.peek().is_none() {
        return false;
    }
    parts.all(|part| {
        let Some(unit) = part.chars().last() else {
            return false;
        };
        matches!(unit, 'h' | 'm' | 's')
            && part.len() > unit.len_utf8()
            && part[..part.len() - unit.len_utf8()]
                .bytes()
                .all(|byte| byte.is_ascii_digit())
    })
}

fn starts_with_any(line: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| line.starts_with(prefix))
}

fn slice_mapped_runs(runs: &[StyleRun], mappings: &[(usize, usize, usize)]) -> Vec<StyleRun> {
    let mut sliced: Vec<StyleRun> = Vec::new();
    for &(source_start, source_end, destination_start) in mappings {
        for run in runs {
            let start = run.start_byte.max(source_start);
            let end = run.end_byte.min(source_end);
            if start >= end {
                continue;
            }
            let mapped = StyleRun {
                start_byte: destination_start + start - source_start,
                end_byte: destination_start + end - source_start,
                foreground: run.foreground,
                background: run.background,
                modifiers: run.modifiers,
            };
            if let Some(previous) = sliced.last_mut()
                && previous.end_byte == mapped.start_byte
                && previous.foreground == mapped.foreground
                && previous.background == mapped.background
                && previous.modifiers == mapped.modifiers
            {
                previous.end_byte = mapped.end_byte;
            } else {
                sliced.push(mapped);
            }
        }
    }
    sliced
}

fn consume_escape(
    bytes: &[u8],
    start: usize,
    style: &mut StyleState,
    runs: &mut StyleRunBuilder,
    output_position: usize,
) -> usize {
    let Some(&kind) = bytes.get(start + 1) else {
        return bytes.len();
    };
    match kind {
        b'[' => consume_csi(bytes, start, style, runs, output_position),
        b']' => consume_osc(bytes, start),
        b'P' | b'_' | b'^' => consume_st_string(bytes, start + 2),
        0x20..=0x2f => consume_escape_with_intermediate(bytes, start + 2),
        _ => (start + 2).min(bytes.len()),
    }
}

fn consume_csi(
    bytes: &[u8],
    start: usize,
    style: &mut StyleState,
    runs: &mut StyleRunBuilder,
    output_position: usize,
) -> usize {
    let mut index = start + 2;
    while let Some(&byte) = bytes.get(index) {
        if (0x40..=0x7e).contains(&byte) {
            if byte == b'm' {
                apply_sgr(&bytes[start + 2..index], style, runs, output_position);
            }
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn consume_osc(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'\x07' {
            return index + 1;
        }
        if byte == b'\x1b' && bytes.get(index + 1) == Some(&b'\\') {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

fn consume_st_string(bytes: &[u8], mut index: usize) -> usize {
    while let Some(&byte) = bytes.get(index) {
        if byte == b'\x1b' && bytes.get(index + 1) == Some(&b'\\') {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

fn consume_escape_with_intermediate(bytes: &[u8], mut index: usize) -> usize {
    while matches!(bytes.get(index), Some(0x20..=0x2f)) {
        index += 1;
    }
    if bytes.get(index).is_some() {
        index + 1
    } else {
        bytes.len()
    }
}

fn apply_sgr(
    parameters: &[u8],
    style: &mut StyleState,
    runs: &mut StyleRunBuilder,
    output_position: usize,
) {
    let Some(parameters) = parse_parameters(parameters) else {
        return;
    };
    let mut next = *style;
    let mut index = 0;
    while index < parameters.len() {
        match parameters[index] {
            0 => next = StyleState::default(),
            1 => next.modifiers.bold = true,
            2 => next.modifiers.dim = true,
            3 => next.modifiers.italic = true,
            4 => next.modifiers.underline = true,
            22 => {
                next.modifiers.bold = false;
                next.modifiers.dim = false;
            }
            23 => next.modifiers.italic = false,
            24 => next.modifiers.underline = false,
            30..=37 | 90..=97 => next.foreground = named_color(parameters[index]),
            39 => next.foreground = None,
            40..=47 | 100..=107 => next.background = named_color(parameters[index]),
            49 => next.background = None,
            38 | 48 => {
                let consumed = color_parameter_len(&parameters[index..]);
                if let Some((value, _)) = extended_color(&parameters[index..]) {
                    if parameters[index] == 38 {
                        next.foreground = Some(value);
                    } else {
                        next.background = Some(value);
                    }
                }
                index += consumed.saturating_sub(1);
            }
            _ => {}
        }
        index += 1;
    }
    runs.set_style(next, output_position);
    *style = next;
}

fn color_parameter_len(parameters: &[u16]) -> usize {
    match parameters.get(1) {
        Some(5) => parameters.len().min(3),
        Some(2) => parameters.len().min(5),
        _ => 1,
    }
}

fn parse_parameters(parameters: &[u8]) -> Option<Vec<u16>> {
    if parameters.is_empty() {
        return Some(vec![0]);
    }
    parameters
        .split(|byte| *byte == b';')
        .map(|part| {
            if part.is_empty() || !part.iter().all(u8::is_ascii_digit) {
                return None;
            }
            std::str::from_utf8(part).ok()?.parse().ok()
        })
        .collect()
}

fn extended_color(parameters: &[u16]) -> Option<(AnsiColor, usize)> {
    match parameters {
        [_, 5, value, ..] => u8::try_from(*value)
            .ok()
            .map(|value| (AnsiColor::Indexed(value), 3)),
        [_, 2, red, green, blue, ..] => Some((
            AnsiColor::Rgb(
                u8::try_from(*red).ok()?,
                u8::try_from(*green).ok()?,
                u8::try_from(*blue).ok()?,
            ),
            5,
        )),
        _ => None,
    }
}

fn named_color(code: u16) -> Option<AnsiColor> {
    Some(match code {
        30 | 40 => AnsiColor::Black,
        31 | 41 => AnsiColor::Red,
        32 | 42 => AnsiColor::Green,
        33 | 43 => AnsiColor::Yellow,
        34 | 44 => AnsiColor::Blue,
        35 | 45 => AnsiColor::Magenta,
        36 | 46 => AnsiColor::Cyan,
        37 | 47 => AnsiColor::White,
        90 | 100 => AnsiColor::BrightBlack,
        91 | 101 => AnsiColor::BrightRed,
        92 | 102 => AnsiColor::BrightGreen,
        93 | 103 => AnsiColor::BrightYellow,
        94 | 104 => AnsiColor::BrightBlue,
        95 | 105 => AnsiColor::BrightMagenta,
        96 | 106 => AnsiColor::BrightCyan,
        97 | 107 => AnsiColor::BrightWhite,
        _ => return None,
    })
}
