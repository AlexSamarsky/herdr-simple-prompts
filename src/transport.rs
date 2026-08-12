use crate::agent::{AgentIdentity, agent_identity};
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

    pub fn submit(&self, text: &str) -> AppResult<()> {
        self.validate_source()?;
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
        let result = self
            .client
            .pane_read_visible(&self.original.pane_id, lines)
            .map_err(|error| AppError::new("source screen", error.to_string()))?;
        result
            .pointer("/read/text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::new("source screen", "Herdr response has no visible text"))
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
