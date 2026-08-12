use crate::style::{AnsiColor, StyleRunBuilder, StyleState, StyledText};

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
                text.push(character);
                index += character.len_utf8();
            }
        }
    }

    StyledText {
        runs: runs.finish(text.len()),
        text,
    }
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
