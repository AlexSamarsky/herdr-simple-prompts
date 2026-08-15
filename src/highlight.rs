//! Syntax colouring for fenced code blocks.
//!
//! Native style capture can only reproduce an answer that fits the captured
//! window, and the window is the visible screen — so long answers always render
//! through the markdown fallback. Without colouring here, the same code block
//! looked syntax-highlighted or flat depending only on how long the surrounding
//! answer happened to be.
//!
//! The palette is measured from a live pane rather than chosen: the agents emit
//! indexed colours, so indexed colours are what we emit.
//!
//! Scanning is per line and deliberately shallow. A block comment or a
//! triple-quoted string that spans lines is coloured only on the lines where it
//! opens; nothing here tries to parse a language, and a wrong guess costs a
//! colour rather than a corrupted line.

use crate::style::AnsiColor;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Language {
    Rust,
    Shell,
    Json,
    Python,
    Script,
    Yaml,
    Toml,
    Diff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TokenKind {
    Comment,
    String,
    Number,
    Keyword,
    Type,
    Function,
    Added,
    Removed,
    Meta,
}

impl TokenKind {
    /// Measured from a live pane: `def`/`return`/`None` blue, the called name
    /// yellow, `str`/`dict` cyan, string literals red, numerals green.
    pub(crate) fn color(self) -> AnsiColor {
        match self {
            Self::Comment | Self::Number | Self::Added => AnsiColor::Indexed(2),
            Self::String | Self::Removed => AnsiColor::Indexed(1),
            Self::Keyword => AnsiColor::Indexed(4),
            Self::Type | Self::Meta => AnsiColor::Indexed(6),
            Self::Function => AnsiColor::Indexed(3),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// Resolves a fence info string to a language, ignoring attributes such as
/// ```` ```rust,ignore ````. An unknown language leaves the block plain.
pub(crate) fn language(info: &str) -> Option<Language> {
    let name = info
        .split(|character: char| character.is_whitespace() || character == ',')
        .next()?
        .to_ascii_lowercase();
    match name.as_str() {
        "rust" | "rs" => Some(Language::Rust),
        "bash" | "sh" | "shell" | "zsh" => Some(Language::Shell),
        "json" | "jsonc" => Some(Language::Json),
        "python" | "py" => Some(Language::Python),
        "javascript" | "js" | "jsx" | "typescript" | "ts" | "tsx" => Some(Language::Script),
        "yaml" | "yml" => Some(Language::Yaml),
        "toml" => Some(Language::Toml),
        "diff" | "patch" => Some(Language::Diff),
        _ => None,
    }
}

/// A diff is coloured by line, not by token: the leading marker decides the
/// whole line.
///
/// The file headers follow the same rule as the body — `---` red, `+++` green —
/// rather than being set apart as metadata, which is how the panes render them.
fn diff_line(line: &str) -> Option<TokenKind> {
    const META: [&str; 4] = ["@@", "diff ", "index ", "=== "];
    if META.iter().any(|prefix| line.starts_with(prefix)) {
        return Some(TokenKind::Meta);
    }
    match line.as_bytes().first() {
        Some(b'+') => Some(TokenKind::Added),
        Some(b'-') => Some(TokenKind::Removed),
        _ => None,
    }
}

pub(crate) fn tokens(language: Language, line: &str) -> Vec<Token> {
    if language == Language::Diff {
        return diff_line(line)
            .map(|kind| {
                vec![Token {
                    start: 0,
                    end: line.len(),
                    kind,
                }]
            })
            .unwrap_or_default();
    }

    let bytes = line.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if comment_starts_at(language, bytes, index) {
            push(&mut tokens, index, line.len(), TokenKind::Comment);
            break;
        }
        let byte = bytes[index];
        if quote_opens(language, byte) {
            let end = string_end(bytes, index);
            push(&mut tokens, index, end, TokenKind::String);
            index = end;
            continue;
        }
        if byte.is_ascii_digit() && !continues_word(bytes, index) {
            let end = number_end(bytes, index);
            push(&mut tokens, index, end, TokenKind::Number);
            index = end;
            continue;
        }
        if starts_word(byte) {
            let end = word_end(bytes, index);
            if let Some((end, kind)) = classify(language, &line[index..end], bytes, index, end) {
                push(&mut tokens, index, end, kind);
            }
            index = end;
            continue;
        }
        index += 1;
    }

    tokens
}

fn push(tokens: &mut Vec<Token>, start: usize, end: usize, kind: TokenKind) {
    if start < end {
        tokens.push(Token { start, end, kind });
    }
}

fn comment_starts_at(language: Language, bytes: &[u8], index: usize) -> bool {
    match language {
        Language::Rust | Language::Script => bytes[index..].starts_with(b"//"),
        Language::Shell | Language::Python | Language::Yaml | Language::Toml => {
            bytes[index] == b'#'
        }
        Language::Json | Language::Diff => false,
    }
}

fn quote_opens(language: Language, byte: u8) -> bool {
    match language {
        Language::Json => byte == b'"',
        Language::Script => matches!(byte, b'"' | b'\'' | b'`'),
        Language::Diff => false,
        Language::Rust | Language::Shell | Language::Python | Language::Yaml | Language::Toml => {
            matches!(byte, b'"' | b'\'')
        }
    }
}

/// End of a quoted run, past its closing quote. An unterminated literal runs to
/// the end of the line rather than swallowing the rest of the block.
fn string_end(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if quote != b'\'' => index += 2,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn number_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'.' || bytes[index] == b'_')
    {
        index += 1;
    }
    index
}

fn starts_word(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn continues_word(bytes: &[u8], index: usize) -> bool {
    index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_')
}

fn word_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_') {
        index += 1;
    }
    index
}

/// Classifies a bare word, returning the token end so a macro can absorb its
/// `!`. Keywords win over calls, so `return(x)` stays a keyword.
fn classify(
    language: Language,
    word: &str,
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Option<(usize, TokenKind)> {
    if keywords(language).contains(&word) {
        return Some((end, TokenKind::Keyword));
    }
    if is_type(language, word) {
        return Some((end, TokenKind::Type));
    }
    if let Some(call_end) = call_end(language, bytes, end) {
        return Some((call_end, TokenKind::Function));
    }
    if structural_key(language, bytes, start, end) {
        return Some((end, TokenKind::Type));
    }
    if builtins(language).contains(&word) {
        return Some((end, TokenKind::Function));
    }
    None
}

/// Whether a bare word is the key of a configuration entry.
///
/// Only a word that opens its line can be one, so a `:` inside a value — the
/// scheme of a URL, say — is not mistaken for a key.
fn structural_key(language: Language, bytes: &[u8], start: usize, end: usize) -> bool {
    let (openers, terminators): (&[u8], &[u8]) = match language {
        Language::Yaml => (b" -", b":"),
        Language::Toml => (b" [.", b"=].".as_slice()),
        _ => return false,
    };
    if !bytes[..start].iter().all(|byte| openers.contains(byte)) {
        return false;
    }
    let mut index = end;
    while bytes.get(index) == Some(&b' ') {
        index += 1;
    }
    bytes
        .get(index)
        .is_some_and(|byte| terminators.contains(byte))
}

fn call_end(language: Language, bytes: &[u8], end: usize) -> Option<usize> {
    if bytes.get(end) == Some(&b'(') {
        return Some(end);
    }
    if language == Language::Rust
        && bytes.get(end) == Some(&b'!')
        && bytes.get(end + 1) == Some(&b'(')
    {
        return Some(end + 1);
    }
    None
}

fn is_type(language: Language, word: &str) -> bool {
    match language {
        Language::Rust | Language::Script => {
            types(language).contains(&word)
                || word
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_uppercase())
        }
        Language::Python => PYTHON_TYPES.contains(&word),
        Language::Shell | Language::Json | Language::Yaml | Language::Toml | Language::Diff => {
            false
        }
    }
}

fn types(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => RUST_TYPES,
        Language::Python => PYTHON_TYPES,
        Language::Script => SCRIPT_TYPES,
        _ => &[],
    }
}

fn keywords(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
            "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "type",
            "unsafe", "use", "where", "while",
        ],
        Language::Shell => &[
            "case", "declare", "do", "done", "elif", "else", "esac", "export", "fi", "for",
            "function", "if", "in", "local", "readonly", "return", "then", "until", "while",
        ],
        Language::Json => &["false", "null", "true"],
        Language::Script => &[
            "as",
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "from",
            "function",
            "if",
            "implements",
            "import",
            "in",
            "instanceof",
            "interface",
            "let",
            "new",
            "null",
            "of",
            "private",
            "public",
            "readonly",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "type",
            "typeof",
            "undefined",
            "var",
            "while",
            "yield",
        ],
        Language::Yaml | Language::Toml => &["false", "no", "null", "off", "on", "true", "yes"],
        Language::Diff => &[],
        Language::Python => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
            "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return",
            "True", "try", "while", "with", "yield",
        ],
    }
}

fn builtins(language: Language) -> &'static [&'static str] {
    match language {
        Language::Shell => &[
            "cd", "echo", "eval", "exec", "exit", "printf", "pwd", "read", "set", "shift",
            "source", "test", "unset",
        ],
        Language::Rust
        | Language::Json
        | Language::Python
        | Language::Script
        | Language::Yaml
        | Language::Toml
        | Language::Diff => &[],
    }
}

const RUST_TYPES: &[&str] = &[
    "bool", "char", "f32", "f64", "i128", "i16", "i32", "i64", "i8", "isize", "str", "u128", "u16",
    "u32", "u64", "u8", "usize",
];

const SCRIPT_TYPES: &[&str] = &[
    "any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown", "void",
];

const PYTHON_TYPES: &[&str] = &[
    "bool",
    "bytes",
    "dict",
    "float",
    "frozenset",
    "int",
    "list",
    "object",
    "set",
    "str",
    "tuple",
    "type",
];

#[cfg(test)]
mod tests {
    use super::{Language, TokenKind, language, tokens};

    fn classified(language: Language, line: &str) -> Vec<(&str, TokenKind)> {
        tokens(language, line)
            .into_iter()
            .map(|token| (&line[token.start..token.end], token.kind))
            .collect()
    }

    /// The exact line measured from a live pane, with the colours it carried.
    #[test]
    fn python_line_matches_the_measured_pane_rendering() {
        assert_eq!(
            classified(
                Language::Python,
                "def call(method: str, params: dict) -> None:"
            ),
            vec![
                ("def", TokenKind::Keyword),
                ("call", TokenKind::Function),
                ("str", TokenKind::Type),
                ("dict", TokenKind::Type),
                ("None", TokenKind::Keyword),
            ],
        );
        assert_eq!(
            classified(
                Language::Python,
                r#"    return {"id": 1, "method": method}"#
            ),
            vec![
                ("return", TokenKind::Keyword),
                ("\"id\"", TokenKind::String),
                ("1", TokenKind::Number),
                ("\"method\"", TokenKind::String),
            ],
        );
    }

    #[test]
    fn rust_macros_absorb_their_bang_and_types_stay_cyan() {
        assert_eq!(
            classified(Language::Rust, r#"    println!("literal {count}");"#),
            vec![
                ("println!", TokenKind::Function),
                ("\"literal {count}\"", TokenKind::String),
            ],
        );
        assert_eq!(
            classified(Language::Rust, "fn main() -> Result<(), Error> {"),
            vec![
                ("fn", TokenKind::Keyword),
                ("main", TokenKind::Function),
                ("Result", TokenKind::Type),
                ("Error", TokenKind::Type),
            ],
        );
        assert_eq!(
            classified(Language::Rust, "    let count: u32 = 42;"),
            vec![
                ("let", TokenKind::Keyword),
                ("u32", TokenKind::Type),
                ("42", TokenKind::Number),
            ],
        );
    }

    #[test]
    fn shell_separates_keywords_from_builtins() {
        assert_eq!(
            classified(Language::Shell, "for name in one two; do"),
            vec![
                ("for", TokenKind::Keyword),
                ("in", TokenKind::Keyword),
                ("do", TokenKind::Keyword),
            ],
        );
        assert_eq!(
            classified(Language::Shell, "  echo \"value: $name\" | grep -c 'x'"),
            vec![
                ("echo", TokenKind::Function),
                ("\"value: $name\"", TokenKind::String),
                ("'x'", TokenKind::String),
            ],
        );
    }

    #[test]
    fn json_colours_literals_and_leaves_everything_else_plain() {
        assert_eq!(
            classified(
                Language::Json,
                r#"{ "key": "value", "number": 42, "flag": true, "nothing": null }"#,
            ),
            vec![
                ("\"key\"", TokenKind::String),
                ("\"value\"", TokenKind::String),
                ("\"number\"", TokenKind::String),
                ("42", TokenKind::Number),
                ("\"flag\"", TokenKind::String),
                ("true", TokenKind::Keyword),
                ("\"nothing\"", TokenKind::String),
                ("null", TokenKind::Keyword),
            ],
        );
    }

    #[test]
    fn comments_run_to_the_end_of_the_line_and_swallow_code_like_text() {
        for (language, line) in [
            (Language::Rust, "// let x: u32 = 1;"),
            (Language::Shell, "# echo \"unterminated"),
            (Language::Python, "# def call():"),
        ] {
            assert_eq!(
                classified(language, line),
                vec![(line, TokenKind::Comment)],
                "{language:?}",
            );
        }
        assert!(tokens(Language::Json, "// not a json comment").is_empty());
    }

    /// A `#` or `//` inside a literal is text, not the start of a comment.
    #[test]
    fn quotes_are_consumed_before_comment_markers() {
        assert_eq!(
            classified(Language::Shell, r#"echo "https://example.test" # tail"#),
            vec![
                ("echo", TokenKind::Function),
                ("\"https://example.test\"", TokenKind::String),
                ("# tail", TokenKind::Comment),
            ],
        );
        assert_eq!(
            classified(Language::Rust, r#"let path = "a // b";"#),
            vec![
                ("let", TokenKind::Keyword),
                ("\"a // b\"", TokenKind::String)
            ],
        );
    }

    #[test]
    fn an_unterminated_literal_stops_at_the_line_end() {
        assert_eq!(
            classified(Language::Python, "value = \"open"),
            vec![("\"open", TokenKind::String)],
        );
    }

    #[test]
    fn identifiers_that_contain_digits_are_not_numbers() {
        assert!(
            tokens(Language::Rust, "let value2 = other_3;")
                .iter()
                .all(|token| token.kind != TokenKind::Number)
        );
    }

    /// A diff is coloured by line: added green, removed red, hunk headers cyan.
    /// The file headers are not set apart — they carry the same marker as the
    /// body and take the same colour, as the panes render them.
    #[test]
    fn diffs_are_coloured_by_their_leading_marker() {
        for (line, kind) in [
            ("+++ b/src/main.rs", Some(TokenKind::Added)),
            ("--- a/src/main.rs", Some(TokenKind::Removed)),
            ("@@ -1,4 +1,6 @@", Some(TokenKind::Meta)),
            ("diff --git a/x b/x", Some(TokenKind::Meta)),
            ("+    let added = 1;", Some(TokenKind::Added)),
            ("-    let removed = 1;", Some(TokenKind::Removed)),
            ("     let untouched = 1;", None),
            ("", None),
        ] {
            let expected = kind.map(|kind| vec![(line, kind)]).unwrap_or_default();
            assert_eq!(classified(Language::Diff, line), expected, "{line:?}");
        }
    }

    #[test]
    fn script_sources_colour_keywords_types_and_templates() {
        assert_eq!(
            classified(Language::Script, "export const total: number = call(1);"),
            vec![
                ("export", TokenKind::Keyword),
                ("const", TokenKind::Keyword),
                ("number", TokenKind::Type),
                ("call", TokenKind::Function),
                ("1", TokenKind::Number),
            ],
        );
        assert_eq!(
            classified(Language::Script, "// a comment"),
            vec![("// a comment", TokenKind::Comment)],
        );
        assert_eq!(
            classified(Language::Script, "const label = `template ${value}`;"),
            vec![
                ("const", TokenKind::Keyword),
                ("`template ${value}`", TokenKind::String),
            ],
        );
    }

    #[test]
    fn configuration_keys_are_coloured_only_when_they_open_a_line() {
        assert_eq!(
            classified(Language::Yaml, "  - name: value # trailing"),
            vec![
                ("name", TokenKind::Type),
                ("# trailing", TokenKind::Comment),
            ],
        );
        assert_eq!(
            classified(Language::Yaml, "url: https://example.test"),
            vec![("url", TokenKind::Type)],
            "a scheme inside a value is not a key",
        );
        assert_eq!(
            classified(Language::Yaml, "enabled: true"),
            vec![("enabled", TokenKind::Type), ("true", TokenKind::Keyword)],
        );
        assert_eq!(
            classified(Language::Toml, "[package]"),
            vec![("package", TokenKind::Type)],
        );
        assert_eq!(
            classified(Language::Toml, "version = \"0.1.0\""),
            vec![
                ("version", TokenKind::Type),
                ("\"0.1.0\"", TokenKind::String)
            ],
        );
    }

    #[test]
    fn info_strings_resolve_aliases_and_ignore_attributes() {
        assert_eq!(language("rust"), Some(Language::Rust));
        assert_eq!(language("rs"), Some(Language::Rust));
        assert_eq!(language("rust,ignore"), Some(Language::Rust));
        assert_eq!(language("BASH"), Some(Language::Shell));
        assert_eq!(language("py"), Some(Language::Python));
        assert_eq!(language("jsonc"), Some(Language::Json));
        assert_eq!(language("ts"), Some(Language::Script));
        assert_eq!(language("tsx"), Some(Language::Script));
        assert_eq!(language("javascript"), Some(Language::Script));
        assert_eq!(language("yml"), Some(Language::Yaml));
        assert_eq!(language("toml"), Some(Language::Toml));
        assert_eq!(language("patch"), Some(Language::Diff));
        assert_eq!(language(""), None);
        assert_eq!(language("brainfuck"), None);
    }
}
