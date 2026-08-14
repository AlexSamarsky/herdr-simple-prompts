use crate::agent::AgentKind;
use crate::native_chrome::{footer_fields, valid_model_label};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusLine {
    pub agent: AgentKind,
    pub model: Option<String>,
    pub cwd: PathBuf,
    pub branch: Option<String>,
    pub usage: Option<String>,
}

pub fn extract_status(kind: AgentKind, visible: &str, known_cwd: PathBuf) -> StatusLine {
    let mut status = StatusLine {
        agent: kind,
        model: None,
        cwd: known_cwd,
        branch: None,
        usage: None,
    };
    let Some(line) = visible.lines().rev().find(|line| !line.trim().is_empty()) else {
        return status;
    };
    let fields = footer_fields(line);
    match kind {
        AgentKind::Codex if fields.len() >= 3 && valid_model_label(fields[0]) => {
            status.model = Some(fields[0].to_owned());
            let usage_fields = fields
                .iter()
                .skip(2)
                .copied()
                .filter(|field| field.contains('%') || field.contains("used"))
                .collect::<Vec<_>>();
            if !usage_fields.is_empty() {
                status.usage = Some(usage_fields.join(" · "));
            }
        }
        AgentKind::Claude if fields.len() >= 2 && valid_model_label(fields[0]) => {
            status.model = Some(fields[0].to_owned());
            if let Some(usage) = fields.iter().copied().find(|field| field.contains('%')) {
                status.usage = Some(usage.to_owned());
            }
        }
        _ => {}
    }
    status
}
