use crate::agent::AgentKind;
use crate::style::{AnsiColor, StyleRun, StyledText, validate_styled_text};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeComposerState {
    Clear,
    OwnedAttachments(usize),
    Occupied,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerAccess {
    Ready,
    Occupied,
    Unknown,
}

impl NativeComposerState {
    pub fn access(self, expected_attachments: usize) -> ComposerAccess {
        match self {
            Self::Clear if expected_attachments == 0 => ComposerAccess::Ready,
            Self::OwnedAttachments(actual) if actual == expected_attachments => {
                ComposerAccess::Ready
            }
            Self::Unknown => ComposerAccess::Unknown,
            Self::Clear | Self::OwnedAttachments(_) | Self::Occupied => ComposerAccess::Occupied,
        }
    }
}

pub fn classify_native_composer(kind: AgentKind, surface: &StyledText) -> NativeComposerState {
    if validate_styled_text(surface).is_err() {
        return NativeComposerState::Unknown;
    }
    let lines = line_ranges(&surface.text);
    let Some(chunks) = (match kind {
        AgentKind::Codex => codex_content(&surface.text, &lines),
        AgentKind::Claude => claude_content(&surface.text, &lines),
    }) else {
        return NativeComposerState::Unknown;
    };

    classify_content(surface, &chunks)
}

#[derive(Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

fn line_ranges(text: &str) -> Vec<LineRange> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(LineRange { start, end: index });
            start = index + 1;
        }
    }
    if start < text.len() || text.ends_with('\n') {
        lines.push(LineRange {
            start,
            end: text.len(),
        });
    }
    lines
}

fn line_text(text: &str, range: LineRange) -> &str {
    &text[range.start..range.end]
}

fn codex_content(text: &str, lines: &[LineRange]) -> Option<Vec<LineRange>> {
    let footer = lines
        .iter()
        .rposition(|line| is_known_footer(line_text(text, *line), AgentKind::Codex))?;
    if lines[footer + 1..]
        .iter()
        .any(|line| !line_text(text, *line).trim().is_empty())
    {
        return None;
    }

    let prompt = (0..footer)
        .rev()
        .find(|index| composer_content_start(line_text(text, lines[*index]), '›').is_some())?;
    let boundary = previous_nonempty_line(text, lines, prompt)?;
    if !is_codex_boundary(line_text(text, lines[boundary])) {
        return None;
    }

    let first = lines[prompt];
    let prefix_len = composer_content_start(line_text(text, first), '›')?;
    let mut chunks = vec![LineRange {
        start: first.start + prefix_len,
        end: first.end,
    }];
    for line in &lines[prompt + 1..footer] {
        let value = line_text(text, *line);
        if value.is_empty() {
            chunks.push(*line);
        } else {
            let content = value.strip_prefix("  ")?;
            chunks.push(LineRange {
                start: line.end - content.len(),
                end: line.end,
            });
        }
    }
    Some(chunks)
}

/// Claude's composer is bounded by a pair of rule lines, and what follows them
/// is a mode hint rather than a footer.
///
/// The shipping build prints `⏵⏵ accept edits on (shift+tab to cycle) · esc to
/// interrupt · ← for agents` there — no model, no working directory. Demanding
/// a `model · cwd` footer therefore never matched, and the overlay treated
/// every Claude pane as unverifiable. Anchor on the rules instead and require
/// only that nothing agent-authored follows them.
fn claude_content(text: &str, lines: &[LineRange]) -> Option<Vec<LineRange>> {
    let close = lines
        .iter()
        .rposition(|line| is_pure_separator(line_text(text, *line), 16))?;
    let trailing = lines[close + 1..]
        .iter()
        .map(|line| line_text(text, *line))
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    if trailing.len() > 1 || trailing.iter().any(|line| line.starts_with("⏺ ")) {
        return None;
    }

    let open = (0..close)
        .rev()
        .find(|index| is_pure_separator(line_text(text, lines[*index]), 16))?;
    let prompt = (open + 1..close)
        .find(|index| composer_content_start(line_text(text, lines[*index]), '❯').is_some())?;
    if lines[open + 1..prompt]
        .iter()
        .any(|line| !line_text(text, *line).trim().is_empty())
    {
        return None;
    }

    let first = lines[prompt];
    let prefix_len = composer_content_start(line_text(text, first), '❯')?;
    let mut chunks = vec![LineRange {
        start: first.start + prefix_len,
        end: first.end,
    }];
    chunks.extend(lines[prompt + 1..close].iter().copied());
    Some(chunks)
}

/// Byte offset of the composer content on a prompt line, if the line is one.
///
/// The separator after the marker is any whitespace, not just an ASCII space:
/// Claude renders an *empty* composer as `❯` followed by U+00A0, so requiring
/// `"❯ "` failed to recognise the composer in the one state where typing into
/// it is safe, and the overlay refused all input.
pub(crate) fn composer_content_start(line: &str, marker: char) -> Option<usize> {
    let rest = line.strip_prefix(marker)?;
    match rest.chars().next() {
        None => Some(marker.len_utf8()),
        Some(character) if character.is_whitespace() => {
            Some(marker.len_utf8() + character.len_utf8())
        }
        Some(_) => None,
    }
}

fn previous_nonempty_line(text: &str, lines: &[LineRange], before: usize) -> Option<usize> {
    (0..before)
        .rev()
        .find(|index| !line_text(text, lines[*index]).trim().is_empty())
}

fn is_codex_boundary(line: &str) -> bool {
    is_pure_separator(line, 8) || is_worked_boundary(line) || is_working_boundary(line)
}

fn is_working_boundary(line: &str) -> bool {
    const PREFIX: &str = "• Working (";
    const SUFFIX: &str = " • esc to interrupt)";
    line.strip_prefix(PREFIX)
        .and_then(|value| value.strip_suffix(SUFFIX))
        .is_some_and(valid_elapsed_label)
}

fn is_worked_boundary(line: &str) -> bool {
    const PREFIX: &str = "─ Worked for ";
    if line.chars().count() < 8 || !line.starts_with(PREFIX) {
        return false;
    }
    let Some((elapsed, suffix)) = line[PREFIX.len()..].rsplit_once(' ') else {
        return false;
    };
    valid_elapsed_label(elapsed)
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

fn is_pure_separator(line: &str, minimum_width: usize) -> bool {
    line.chars().count() >= minimum_width && line.chars().all(|character| character == '─')
}

fn is_known_footer(line: &str, kind: AgentKind) -> bool {
    let fields = line
        .split('·')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let [model, cwd, ..] = fields.as_slice() else {
        return false;
    };
    let model_is_known = match kind {
        AgentKind::Codex => model.starts_with("gpt-") && valid_model_label(model),
        AgentKind::Claude => {
            model
                .split_ascii_whitespace()
                .any(|word| matches!(word, "Claude" | "Opus"))
                && valid_model_label(model)
        }
    };
    model_is_known && (*cwd == "~" || cwd.starts_with("~/") || cwd.starts_with('/'))
}

fn valid_model_label(model: &str) -> bool {
    !model.is_empty()
        && model.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character.is_ascii_whitespace()
                || matches!(character, '-' | '_' | '.')
        })
}

fn classify_content(surface: &StyledText, chunks: &[LineRange]) -> NativeComposerState {
    let mut content = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        if index > 0 {
            content.push('\n');
        }
        content.push_str(&surface.text[chunk.start..chunk.end]);
    }
    if content.trim().is_empty() {
        return NativeComposerState::Clear;
    }
    if let Some(count) = exact_attachment_count(&content) {
        return NativeComposerState::OwnedAttachments(count);
    }
    if chunks.iter().all(|chunk| {
        surface.text[chunk.start..chunk.end]
            .char_indices()
            .filter(|(_, character)| !character.is_whitespace())
            .all(|(offset, character)| {
                placeholder_style_at(
                    &surface.runs,
                    chunk.start + offset,
                    chunk.start + offset + character.len_utf8(),
                )
            })
    }) {
        return NativeComposerState::Clear;
    }
    NativeComposerState::Occupied
}

fn placeholder_style_at(runs: &[StyleRun], start: usize, end: usize) -> bool {
    runs.iter().any(|run| {
        run.start_byte <= start
            && run.end_byte >= end
            && ((run.modifiers.dim && run.foreground.is_none())
                || matches!(
                    run.foreground,
                    Some(AnsiColor::BrightBlack | AnsiColor::Indexed(8))
                ))
    })
}

fn exact_attachment_count(content: &str) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut index = 0;
    let mut count = 0;
    while index < bytes.len() {
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if index == bytes.len() {
            break;
        }
        let prefix = b"[Image #";
        if !bytes[index..].starts_with(prefix) {
            return None;
        }
        index += prefix.len();
        let digits_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == digits_start || bytes.get(index) != Some(&b']') {
            return None;
        }
        index += 1;
        if bytes
            .get(index)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            return None;
        }
        count += 1;
    }
    (count > 0).then_some(count)
}
