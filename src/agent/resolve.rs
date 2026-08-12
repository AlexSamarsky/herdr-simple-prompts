use super::AgentKind;
use crate::{AppError, AppResult};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
    let mut visited = HashSet::new();
    visit_files(&root, &root, &mut visited, &mut |path| {
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

fn visit_files(
    path: &Path,
    allowed_root: &Path,
    visited: &mut HashSet<PathBuf>,
    visitor: &mut impl FnMut(&Path),
) -> AppResult<()> {
    let resolved = std::fs::canonicalize(path).map_err(|error| {
        AppError::new(
            "transcript",
            format!("cannot resolve {}: {error}", path.display()),
        )
    })?;
    if !resolved.starts_with(allowed_root) {
        return Ok(());
    }
    let metadata = std::fs::metadata(&resolved).map_err(|error| {
        AppError::new(
            "transcript",
            format!("cannot inspect {}: {error}", resolved.display()),
        )
    })?;
    if metadata.is_file() {
        visitor(&resolved);
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    if !visited.insert(resolved.clone()) {
        return Ok(());
    }
    for entry in std::fs::read_dir(&resolved).map_err(|error| {
        AppError::new(
            "transcript",
            format!("cannot read {}: {error}", resolved.display()),
        )
    })? {
        let entry = entry.map_err(|error| AppError::new("transcript", error.to_string()))?;
        visit_files(&entry.path(), allowed_root, visited, visitor)?;
    }
    Ok(())
}
