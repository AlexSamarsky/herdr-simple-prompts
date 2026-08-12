#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsiColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct StyleModifiers {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct StyleRun {
    pub start_byte: usize,
    pub end_byte: usize,
    pub foreground: Option<AnsiColor>,
    pub background: Option<AnsiColor>,
    pub modifiers: StyleModifiers,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessagePresentation {
    Plain,
    NativeAnsi(Vec<StyleRun>),
    MarkdownFallback,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StyledText {
    pub text: String,
    pub runs: Vec<StyleRun>,
}

pub fn validate_style_runs(text: &str, runs: &[StyleRun]) -> Result<(), String> {
    let mut previous_end = 0;
    for (index, run) in runs.iter().enumerate() {
        if run.start_byte >= run.end_byte {
            return Err(format!("style run {index} is empty or reversed"));
        }
        if run.end_byte > text.len() {
            return Err(format!("style run {index} extends past canonical text"));
        }
        if !text.is_char_boundary(run.start_byte) || !text.is_char_boundary(run.end_byte) {
            return Err(format!(
                "style run {index} does not align to UTF-8 boundaries"
            ));
        }
        if run.start_byte < previous_end {
            return Err(format!("style run {index} overlaps a preceding run"));
        }
        previous_end = run.end_byte;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StyleState {
    pub foreground: Option<AnsiColor>,
    pub background: Option<AnsiColor>,
    pub modifiers: StyleModifiers,
}

pub(crate) struct StyleRunBuilder {
    current: StyleState,
    start_byte: usize,
    runs: Vec<StyleRun>,
}

impl StyleRunBuilder {
    pub(crate) fn new() -> Self {
        Self {
            current: StyleState::default(),
            start_byte: 0,
            runs: Vec::new(),
        }
    }

    pub(crate) fn set_style(&mut self, style: StyleState, position: usize) {
        if self.current == style {
            return;
        }
        self.close(position);
        self.current = style;
        self.start_byte = position;
    }

    pub(crate) fn finish(mut self, position: usize) -> Vec<StyleRun> {
        self.close(position);
        self.runs
    }

    fn close(&mut self, end_byte: usize) {
        if self.start_byte == end_byte || self.current == StyleState::default() {
            return;
        }
        let run = StyleRun {
            start_byte: self.start_byte,
            end_byte,
            foreground: self.current.foreground,
            background: self.current.background,
            modifiers: self.current.modifiers,
        };
        if let Some(previous) = self.runs.last_mut()
            && previous.end_byte == run.start_byte
            && previous.foreground == run.foreground
            && previous.background == run.background
            && previous.modifiers == run.modifiers
        {
            previous.end_byte = run.end_byte;
        } else {
            self.runs.push(run);
        }
    }
}
