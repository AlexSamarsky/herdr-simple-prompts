use crate::agent::{AgentIdentity, AgentStatus, agent_identity};
use crate::ansi::sanitize_ansi;
use crate::composer::{ComposerAccess, classify_native_composer};
use crate::herdr::HerdrClient;
use crate::{AppError, AppResult};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

pub struct AgentTransport {
    client: HerdrClient,
    original: AgentIdentity,
}

impl AgentTransport {
    pub fn new(client: HerdrClient, original: AgentIdentity) -> Self {
        Self { client, original }
    }

    pub fn identity(&self) -> &AgentIdentity {
        &self.original
    }

    pub fn submit(&self, text: &str, expected_attachments: usize) -> AppResult<()> {
        self.validate_source()?;
        let ansi = self
            .client
            .pane_read_visible_ansi(&self.original.pane_id, 200)
            .map_err(|_| {
                AppError::new(
                    "send prompt",
                    "cannot verify native composer is safe to submit; prefix+m to return",
                )
            })?;
        let surface = sanitize_ansi(&ansi);
        match classify_native_composer(self.original.kind, &surface).access(expected_attachments) {
            ComposerAccess::Ready => {}
            ComposerAccess::Occupied => {
                return Err(AppError::new(
                    "send prompt",
                    "native composer contains unsent input; prefix+m to return",
                ));
            }
            ComposerAccess::Unknown => {
                return Err(AppError::new(
                    "send prompt",
                    "cannot verify native composer is safe to submit; prefix+m to return",
                ));
            }
        }
        self.client
            .agent_prompt(&self.original.pane_id, text)
            .map_err(|error| AppError::new("send prompt", error.to_string()))?;
        Ok(())
    }

    pub fn interrupt(&self) -> AppResult<()> {
        let current = self.validate_source()?;
        if !current.status.is_working() {
            return Err(AppError::new("interrupt", "agent is not working"));
        }
        self.client
            .agent_send_keys(&self.original.pane_id, &["esc"])
            .map_err(|error| AppError::new("interrupt", error.to_string()))?;
        Ok(())
    }

    pub fn forward_interaction_text(&self, text: &str) -> AppResult<()> {
        self.validate_blocked_source()?;
        self.client
            .pane_send_input(&self.original.pane_id, Some(text), &[])
            .map_err(|error| AppError::new("native interaction", error.to_string()))?;
        Ok(())
    }

    pub fn forward_interaction_key(&self, key: &str) -> AppResult<()> {
        self.validate_blocked_source()?;
        self.client
            .pane_send_input(&self.original.pane_id, None, &[key])
            .map_err(|error| AppError::new("native interaction", error.to_string()))?;
        Ok(())
    }

    pub fn forward_local_image_paste(&self) -> AppResult<()> {
        let before = self.image_marker_count()?;
        self.validate_source()?;
        self.client
            .agent_send_keys(&self.original.pane_id, &["ctrl+v"])
            .map_err(|error| AppError::new("image paste", error.to_string()))?;
        self.verify_new_image_marker(before)
    }

    pub fn forward_staged_image(&self, path: &Path) -> AppResult<()> {
        let text = path
            .to_str()
            .ok_or_else(|| AppError::new("image paste", "image path is not UTF-8"))?;
        let before = self.image_marker_count()?;
        self.validate_source()?;
        self.client
            .pane_send_input(&self.original.pane_id, Some(text), &[])
            .map_err(|error| AppError::new("image paste", error.to_string()))?;
        self.verify_new_image_marker(before)
    }

    pub fn visible_source(&self, lines: u16) -> AppResult<String> {
        self.validate_source()?;
        self.client
            .pane_read_visible_text(&self.original.pane_id, u32::from(lines))
            .map_err(|error| AppError::new("source screen", error.to_string()))
    }

    pub fn recent_unwrapped_ansi(&self, lines: u32) -> AppResult<String> {
        self.validate_source()?;
        self.client
            .agent_read_recent_unwrapped_ansi(&self.original.pane_id, lines)
            .map_err(|error| AppError::new("capture final style", error.to_string()))
    }

    pub fn visible_source_ansi(&self, lines: u32) -> AppResult<String> {
        self.validate_source()?;
        self.read_visible_source_ansi(lines)
    }

    pub(crate) fn read_visible_source_ansi(&self, lines: u32) -> AppResult<String> {
        self.client
            .pane_read_visible_ansi(&self.original.pane_id, lines)
            .map_err(|error| AppError::new("source screen", error.to_string()))
    }

    pub fn refresh_identity(&self) -> AppResult<AgentIdentity> {
        self.validate_source()
    }

    fn validate_source(&self) -> AppResult<AgentIdentity> {
        let current = agent_identity(&self.client, &self.original.pane_id)?;
        if current.kind != self.original.kind || current.session_id != self.original.session_id {
            return Err(AppError::new(
                "agent",
                "source agent session changed; reopen Simple Prompts",
            ));
        }
        Ok(current)
    }

    fn validate_blocked_source(&self) -> AppResult<AgentIdentity> {
        let current = self.validate_source()?;
        if current.status != AgentStatus::Blocked {
            return Err(AppError::new(
                "native interaction",
                "source agent is no longer blocked",
            ));
        }
        Ok(current)
    }

    fn image_marker_count(&self) -> AppResult<usize> {
        Ok(self.visible_source(20)?.matches("[Image #").count())
    }

    fn verify_new_image_marker(&self, before: usize) -> AppResult<()> {
        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            if self.image_marker_count()? > before {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(40));
        }
        Err(AppError::new(
            "image paste",
            "native agent did not confirm the image attachment",
        ))
    }
}
