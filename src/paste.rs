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
    use super::{canonicalize_compact_markers, fingerprint, marker_counts};

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
