use super::time::record_timestamp_ms;
use crate::model::{Attachment, ConversationEvent, Message};
use crate::{AppError, AppResult};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct CodexAdapter;

impl CodexAdapter {
    pub fn ingest_line(
        &mut self,
        line_number: u64,
        line: &str,
    ) -> AppResult<Option<ConversationEvent>> {
        let record: Value = serde_json::from_str(line).map_err(|error| {
            AppError::new(
                "Codex transcript",
                format!("invalid JSON on line {line_number}: {error}"),
            )
        })?;
        Ok(self.ingest_value(line_number, &record))
    }

    pub fn ingest_value(&mut self, line_number: u64, record: &Value) -> Option<ConversationEvent> {
        if record.get("type").and_then(Value::as_str) != Some("event_msg")
            || truthy(record, "is_subagent")
            || record.get("subagent_id").is_some()
        {
            return None;
        }
        let payload = record.get("payload")?;
        if truthy(payload, "is_subagent") || payload.get("subagent_id").is_some() {
            return None;
        }
        match payload.get("type").and_then(Value::as_str) {
            Some("user_message") => {
                parse_user(line_number, record, payload).map(ConversationEvent::User)
            }
            Some("agent_message")
                if payload.get("phase").and_then(Value::as_str) == Some("final_answer") =>
            {
                parse_final(line_number, record, payload).map(ConversationEvent::Final)
            }
            _ => None,
        }
    }
}

fn parse_user(line_number: u64, record: &Value, payload: &Value) -> Option<Message> {
    let text = payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let attachments = payload
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .enumerate()
        .map(|(index, image)| attachment(index, image))
        .collect::<Vec<_>>();
    if text.trim().is_empty() && attachments.is_empty() {
        return None;
    }
    Some(Message {
        stable_id: stable_id(line_number, payload),
        text,
        presentation: crate::style::MessagePresentation::Plain,
        attachments,
        timestamp_ms: record_timestamp_ms(record),
    })
}

fn parse_final(line_number: u64, record: &Value, payload: &Value) -> Option<Message> {
    let text = payload.get("message").and_then(Value::as_str)?.to_owned();
    if text.trim().is_empty() {
        return None;
    }
    Some(Message::text(
        stable_id(line_number, payload),
        text,
        record_timestamp_ms(record),
    ))
}

fn stable_id(line_number: u64, payload: &Value) -> String {
    payload
        .get("item_id")
        .or_else(|| payload.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("codex:{line_number}"))
}

fn attachment(index: usize, image: &str) -> Attachment {
    let path = PathBuf::from(image);
    let display = Path::new(image)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(image)
        .to_owned();
    Attachment {
        id: format!("codex-image-{}", index + 1),
        display,
        native_path: Some(path),
    }
}

fn truthy(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}
