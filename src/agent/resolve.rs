use super::AgentKind;
use crate::{AppError, AppResult};
use std::path::{Path, PathBuf};

/// Transcript roots are two or three levels deep; the bound stops a runaway
/// walk from recursing through an unrelated tree that was placed there.
const MAX_TRANSCRIPT_DEPTH: usize = 8;

#[derive(Clone, Debug)]
pub struct AgentPaths {
    pub home: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub claude_config: Option<PathBuf>,
}

impl AgentPaths {
    pub fn new(home: PathBuf, codex_home: Option<PathBuf>, claude_config: Option<PathBuf>) -> Self {
        Self {
            home,
            codex_home,
            claude_config,
        }
    }

    pub fn from_env() -> AppResult<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| AppError::new("transcript", "HOME is not set"))?;
        Ok(Self {
            codex_home: std::env::var_os("CODEX_HOME").map(PathBuf::from),
            claude_config: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            home,
        })
    }

    fn root_for(&self, kind: AgentKind) -> PathBuf {
        match kind {
            AgentKind::Codex => self
                .codex_home
                .clone()
                .unwrap_or_else(|| self.home.join(".codex"))
                .join("sessions"),
            AgentKind::Claude => self
                .claude_config
                .clone()
                .unwrap_or_else(|| self.home.join(".claude"))
                .join("projects"),
        }
    }
}

pub fn resolve_transcript(
    kind: AgentKind,
    session_id: &str,
    paths: &AgentPaths,
) -> AppResult<PathBuf> {
    validate_session_id(session_id)?;
    let configured_root = paths.root_for(kind);
    let root = std::fs::canonicalize(&configured_root).map_err(|error| {
        AppError::new(
            "transcript",
            format!("cannot resolve {}: {error}", configured_root.display()),
        )
    })?;
    let mut matches = Vec::new();
    visit_files(&root, 0, &mut |path| {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let matches_session = match kind {
            AgentKind::Codex => {
                file_name == format!("{session_id}.jsonl")
                    || file_name.ends_with(&format!("-{session_id}.jsonl"))
            }
            AgentKind::Claude => file_name == format!("{session_id}.jsonl"),
        };
        if matches_session {
            matches.push(path.to_owned());
        }
    })?;
    matches.sort();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(AppError::new(
            "transcript",
            format!(
                "no {kind} transcript for session {session_id} under {}",
                root.display()
            ),
        )),
        _ => Err(AppError::new(
            "transcript",
            format!("multiple {kind} transcripts match session {session_id}"),
        )),
    }
}

fn validate_session_id(session_id: &str) -> AppResult<()> {
    let valid = !session_id.is_empty()
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            "transcript",
            "native session id contains unsupported characters",
        ))
    }
}

/// Walks the transcript root without following symbolic links.
///
/// Refusing to follow links keeps the walk inside the configured root without a
/// `canonicalize` syscall per entry — the previous shape resolved every file in
/// `~/.claude/projects`, which is the entire session history, on every start of
/// the overlay.
fn visit_files(directory: &Path, depth: usize, visitor: &mut impl FnMut(&Path)) -> AppResult<()> {
    if depth > MAX_TRANSCRIPT_DEPTH {
        return Ok(());
    }
    let entries = std::fs::read_dir(directory).map_err(|error| {
        AppError::new(
            "transcript",
            format!("cannot read {}: {error}", directory.display()),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| AppError::new("transcript", error.to_string()))?;
        let file_type = entry.file_type().map_err(|error| {
            AppError::new(
                "transcript",
                format!("cannot inspect {}: {error}", entry.path().display()),
            )
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            visit_files(&path, depth.saturating_add(1), visitor)?;
        } else if file_type.is_file() {
            visitor(&path);
        }
    }
    Ok(())
}
