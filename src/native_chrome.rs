//! Shared recognition of the native Codex and Claude terminal chrome.
//!
//! The footer, separators and elapsed labels are parsed the same way by the
//! ANSI capture path, the composer classifier and the status line. Keeping one
//! implementation here avoids the three copies drifting apart: a footer rule
//! that only knows some model names silently disables capture in one place and
//! blocks the composer in another.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LineRange {
    pub start: usize,
    pub end: usize,
}

pub(crate) fn line_ranges(text: &str) -> Vec<LineRange> {
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

pub(crate) fn line_text(text: &str, range: LineRange) -> &str {
    &text[range.start..range.end]
}

pub(crate) fn is_pure_separator(line: &str, minimum_width: usize) -> bool {
    line.chars().count() >= minimum_width && line.chars().all(|character| character == '─')
}

pub(crate) fn valid_elapsed_label(label: &str) -> bool {
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

/// A model chip is any short label the agent could print for its own model.
///
/// Matching known model names instead would break on every rename: a Claude
/// pane running Sonnet or Haiku, or a Codex pane on a model that is not named
/// `gpt-*`, would look like "no footer at all" to every caller.
pub(crate) fn valid_model_label(model: &str) -> bool {
    !model.is_empty()
        && model.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || character.is_ascii_whitespace()
                || matches!(character, '-' | '_' | '.')
        })
}

pub(crate) fn footer_fields(line: &str) -> Vec<&str> {
    line.split('·')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect()
}

fn is_workdir_field(field: &str) -> bool {
    field == "~" || field.starts_with("~/") || field.starts_with('/')
}

/// Returns the model chip of a native footer line, if the line is one.
///
/// A footer is recognised structurally: a model chip first, and a working
/// directory in any later field.
pub(crate) fn footer_model(line: &str) -> Option<&str> {
    let fields = footer_fields(line);
    let [model, rest @ ..] = fields.as_slice() else {
        return None;
    };
    if !valid_model_label(model) {
        return None;
    }
    rest.iter()
        .any(|field| is_workdir_field(field))
        .then_some(*model)
}

pub(crate) fn is_known_footer(line: &str) -> bool {
    footer_model(line).is_some()
}

#[cfg(test)]
mod tests {
    use super::{footer_model, is_known_footer, line_ranges, valid_model_label};

    #[test]
    fn footers_are_recognised_for_any_model_chip() {
        for (line, model) in [
            ("Claude Opus · /repo", "Claude Opus"),
            ("Sonnet 4.5 · /repo", "Sonnet 4.5"),
            ("Haiku 4.5 · ~/projects/demo", "Haiku 4.5"),
            ("claude-sonnet-4-5 · ~", "claude-sonnet-4-5"),
            (
                "gpt-5.6-sol xhigh · /repo · weekly 75% left",
                "gpt-5.6-sol xhigh",
            ),
            ("o4-mini · /repo · 12% left", "o4-mini"),
        ] {
            assert_eq!(footer_model(line), Some(model), "footer: {line:?}");
        }
    }

    #[test]
    fn ordinary_answer_lines_are_not_footers() {
        for line in [
            "",
            "just prose",
            "prose · more prose",
            "· /repo",
            "emoji 🚀 model · /repo",
            "see /repo for details",
        ] {
            assert!(!is_known_footer(line), "must not be a footer: {line:?}");
        }
    }

    #[test]
    fn model_labels_reject_decorated_text() {
        assert!(valid_model_label("Sonnet 4.5"));
        assert!(!valid_model_label(""));
        assert!(!valid_model_label("❯ prompt"));
    }

    #[test]
    fn line_ranges_keep_the_trailing_empty_line() {
        let ranges = line_ranges("a\nb\n");
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[2].start, ranges[2].end);
    }
}
