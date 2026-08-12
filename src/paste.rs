pub const LARGE_PASTE_CHARS: usize = 1_000;

pub fn large_paste_marker(character_count: usize) -> String {
    format!("[Pasted Content · {character_count} chars]")
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PasteRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub character_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
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
        if complete_text.len() != self.complete_len
            || fingerprint(complete_text) != self.fingerprint
        {
            return None;
        }

        let mut previous_end = 0;
        for (index, range) in self.paste_ranges.iter().enumerate() {
            if range.start_byte >= range.end_byte
                || range.end_byte > complete_text.len()
                || !complete_text.is_char_boundary(range.start_byte)
                || !complete_text.is_char_boundary(range.end_byte)
                || (index > 0 && range.start_byte < previous_end)
            {
                return None;
            }
            previous_end = range.end_byte;
        }

        let mut output = complete_text.to_owned();
        for range in self.paste_ranges.iter().rev() {
            output.replace_range(
                range.start_byte..range.end_byte,
                &large_paste_marker(range.character_count),
            );
        }
        Some(output)
    }
}

pub fn fingerprint(text: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    text.as_bytes().iter().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

pub fn marker_counts(text: &str) -> Vec<usize> {
    let mut counts = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find('[') {
        let start = cursor + relative_start;
        if let Some((end, count)) = recognized_marker_at(text, start) {
            counts.push(count);
            cursor = end;
        } else {
            cursor = start + 1;
        }
    }

    counts
}

pub fn canonicalize_compact_markers(text: &str) -> String {
    let mut canonical = String::with_capacity(text.len());
    let mut copied_through = 0;
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find('[') {
        let start = cursor + relative_start;
        if let Some((end, count)) = recognized_marker_at(text, start) {
            canonical.push_str(&text[copied_through..start]);
            canonical.push_str(&large_paste_marker(count));
            copied_through = end;
            cursor = end;
        } else {
            cursor = start + 1;
        }
    }

    canonical.push_str(&text[copied_through..]);
    canonical
}

fn recognized_marker_at(text: &str, start: usize) -> Option<(usize, usize)> {
    const PREFIX: &str = "[Pasted Content ";
    const DOTTED_SEPARATOR: &str = "· ";
    const SUFFIX: &str = " chars]";

    let remainder = text.get(start..)?.strip_prefix(PREFIX)?;
    let remainder = remainder
        .strip_prefix(DOTTED_SEPARATOR)
        .unwrap_or(remainder);
    let digit_count = remainder
        .as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }

    let (digits, remainder) = remainder.split_at(digit_count);
    let suffix_remainder = remainder.strip_prefix(SUFFIX)?;
    let count = digits.parse().ok()?;
    let consumed = text.len() - suffix_remainder.len() - start;
    Some((start + consumed, count))
}

#[cfg(test)]
mod tests {
    use super::{
        CompactPromptOverride, PasteRange, canonicalize_compact_markers, fingerprint, marker_counts,
    };

    #[test]
    fn compact_override_rejects_overlapping_paste_ranges() {
        let source = "abcdef";
        let summary = CompactPromptOverride::new(
            "session-1",
            "native-1",
            source,
            vec![
                PasteRange {
                    start_byte: 1,
                    end_byte: 4,
                    character_count: 3,
                },
                PasteRange {
                    start_byte: 3,
                    end_byte: 5,
                    character_count: 2,
                },
            ],
        );

        assert_eq!(summary.compact_text(source), None);
    }

    #[test]
    fn compact_override_rejects_empty_paste_ranges() {
        let source = "abcdef";
        let summary = CompactPromptOverride {
            session_id: "session-1".into(),
            stable_id: "native-1".into(),
            complete_len: source.len(),
            fingerprint: fingerprint(source),
            paste_ranges: vec![PasteRange {
                start_byte: 3,
                end_byte: 3,
                character_count: 0,
            }],
        };

        assert_eq!(summary.compact_text(source), None);
    }

    #[test]
    fn fingerprint_uses_standard_fnv_1a_vectors() {
        assert_eq!(fingerprint(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fingerprint("hello"), 0xa430_d846_80aa_bd0b);
    }

    #[test]
    fn canonicalization_preserves_all_surrounding_text() {
        let source = "before 🌍\n[Pasted Content 0012 chars]\nafter";

        assert_eq!(
            canonicalize_compact_markers(source),
            "before 🌍\n[Pasted Content · 12 chars]\nafter"
        );
        assert_eq!(marker_counts(source), vec![12]);
    }

    #[test]
    fn canonicalization_normalizes_only_exact_native_bracketed_markers() {
        let source = concat!(
            "[Pasted Content · 7 chars] ",
            "[Pasted Content 8 chars] ",
            "Pasted Content 9 chars ",
            "[Pasted content 10 chars] ",
            "[Pasted Content 11 char] ",
            "[Pasted Content  12 chars]"
        );

        assert_eq!(
            canonicalize_compact_markers(source),
            concat!(
                "[Pasted Content · 7 chars] ",
                "[Pasted Content · 8 chars] ",
                "Pasted Content 9 chars ",
                "[Pasted content 10 chars] ",
                "[Pasted Content 11 char] ",
                "[Pasted Content  12 chars]"
            )
        );
        assert_eq!(marker_counts(source), vec![7, 8]);
    }
}
