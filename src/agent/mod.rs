pub mod claude;
pub mod codex;
pub mod follower;
mod resolve;

pub use resolve::{AgentPaths, resolve_transcript};

use crate::herdr::HerdrClient;
use crate::{AppError, AppResult};
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;

pub trait TranscriptAdapter: Send {
    fn ingest_line(
        &mut self,
        line_number: u64,
        line: &str,
    ) -> AppResult<Vec<crate::model::ConversationEvent>>;
    fn reset(&mut self);
}

impl TranscriptAdapter for codex::CodexAdapter {
    fn ingest_line(
        &mut self,
        line_number: u64,
        line: &str,
    ) -> AppResult<Vec<crate::model::ConversationEvent>> {
        Ok(codex::CodexAdapter::ingest_line(self, line_number, line)?
            .into_iter()
            .collect())
    }

    fn reset(&mut self) {}
}

impl TranscriptAdapter for claude::ClaudeAdapter {
    fn ingest_line(
        &mut self,
        line_number: u64,
        line: &str,
    ) -> AppResult<Vec<crate::model::ConversationEvent>> {
        claude::ClaudeAdapter::ingest_line(self, line_number, line)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentKind {
    Codex,
    Claude,
}

impl fmt::Display for AgentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codex => formatter.write_str("Codex"),
            Self::Claude => formatter.write_str("Claude"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    pub fn is_working(self) -> bool {
        self == Self::Working
    }
}

impl From<Option<&str>> for AgentStatus {
    fn from(value: Option<&str>) -> Self {
        match value {
            Some("idle") => Self::Idle,
            Some("working") => Self::Working,
            Some("blocked") => Self::Blocked,
            Some("done") => Self::Done,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentIdentity {
    pub pane_id: String,
    pub kind: AgentKind,
    pub session_id: String,
    pub cwd: PathBuf,
    pub status: AgentStatus,
}

pub fn agent_identity(client: &HerdrClient, pane_id: &str) -> AppResult<AgentIdentity> {
    let result = client
        .agent_get(pane_id)
        .map_err(|error| AppError::new("agent", error.to_string()))?;
    let agent = result
        .get("agent")
        .ok_or_else(|| AppError::new("agent", "Herdr response has no agent record"))?;
    parse_identity(agent, pane_id)
}

fn parse_identity(agent: &Value, expected_pane_id: &str) -> AppResult<AgentIdentity> {
    let pane_id = required_string(agent, "pane_id")?;
    if pane_id != expected_pane_id {
        return Err(AppError::new(
            "agent",
            format!("Herdr returned pane {pane_id} for {expected_pane_id}"),
        ));
    }
    let session = agent
        .get("agent_session")
        .ok_or_else(|| AppError::new("agent", "native agent session is unavailable"))?;
    if session.get("kind").and_then(Value::as_str) != Some("id") {
        return Err(AppError::new(
            "agent",
            "native agent session is not id-based",
        ));
    }
    let kind = match session.get("agent").and_then(Value::as_str) {
        Some("codex") => AgentKind::Codex,
        Some("claude") => AgentKind::Claude,
        Some(other) => {
            return Err(AppError::new(
                "agent",
                format!("unsupported agent: {other}"),
            ));
        }
        None => return Err(AppError::new("agent", "native agent kind is unavailable")),
    };
    let session_id = required_string(session, "value")?;
    let cwd = agent
        .get("foreground_cwd")
        .and_then(Value::as_str)
        .or_else(|| agent.get("cwd").and_then(Value::as_str))
        .map(PathBuf::from)
        .ok_or_else(|| AppError::new("agent", "agent working directory is unavailable"))?;
    Ok(AgentIdentity {
        pane_id,
        kind,
        session_id,
        cwd,
        status: AgentStatus::from(agent.get("agent_status").and_then(Value::as_str)),
    })
}

fn required_string(value: &Value, field: &'static str) -> AppResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::new("agent", format!("missing {field}")))
}
