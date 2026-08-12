use crate::agent::AgentStatus;
use crate::model::{Attachment, Delivery, Message, Turn};
use crate::status::StatusLine;
use std::time::Instant;

const RECONCILE_WINDOW_MS: u64 = 30_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    PromptSubmitted {
        local_id: String,
        text: String,
        attachments: Vec<Attachment>,
        at_ms: u64,
    },
    NativeUser(Message),
    NativeFinal(Message),
    TranscriptReloaded,
    TranscriptReplayComplete,
    SendFailed {
        local_id: String,
        reason: String,
    },
}

pub struct AppState {
    pub turns: Vec<Turn>,
    pub draft: String,
    pub draft_attachments: Vec<Attachment>,
    pub pending_attachments: Vec<Attachment>,
    pub agent_status: AgentStatus,
    pub working_since: Option<Instant>,
    pub status_line: Option<StatusLine>,
    pub connection_error: Option<String>,
    pub transcript_error: Option<String>,
    pub send_error: Option<String>,
    pub input_enabled: bool,
    pub scroll_from_bottom: u16,
    #[doc(hidden)]
    pub replay_insert_at: Option<usize>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            turns: Vec::new(),
            draft: String::new(),
            draft_attachments: Vec::new(),
            pending_attachments: Vec::new(),
            agent_status: AgentStatus::Unknown,
            working_since: None,
            status_line: None,
            connection_error: None,
            transcript_error: None,
            send_error: None,
            input_enabled: true,
            scroll_from_bottom: 0,
            replay_insert_at: None,
        }
    }
}

impl AppState {
    pub fn apply(&mut self, event: AppEvent) {
        match event {
            AppEvent::PromptSubmitted {
                local_id,
                text,
                attachments,
                at_ms,
            } => {
                self.draft.clear();
                self.draft_attachments.clear();
                self.turns.push(Turn {
                    prompt: Message {
                        stable_id: local_id.clone(),
                        text,
                        attachments,
                        timestamp_ms: Some(at_ms),
                    },
                    final_answer: None,
                    delivery: Delivery::Optimistic {
                        local_id,
                        submitted_at_ms: at_ms,
                    },
                });
            }
            AppEvent::NativeUser(message) => self.reconcile_user(message),
            AppEvent::NativeFinal(message) => {
                if let Some(turn) =
                    self.turns.iter_mut().rev().find(|turn| {
                        turn.delivery == Delivery::Native && turn.final_answer.is_none()
                    })
                {
                    turn.final_answer = Some(message);
                }
            }
            AppEvent::TranscriptReloaded => {
                self.turns
                    .retain(|turn| !matches!(turn.delivery, Delivery::Native));
                self.replay_insert_at = Some(0);
            }
            AppEvent::TranscriptReplayComplete => self.replay_insert_at = None,
            AppEvent::SendFailed { local_id, reason } => {
                if let Some(turn) = self.turns.iter_mut().find(|turn| {
                    matches!(
                        &turn.delivery,
                        Delivery::Optimistic { local_id: candidate, .. } if candidate == &local_id
                    )
                }) {
                    self.draft.clone_from(&turn.prompt.text);
                    self.draft_attachments.clone_from(&turn.prompt.attachments);
                    turn.delivery = Delivery::Failed { reason };
                }
            }
        }
    }

    pub fn visible_error(&self) -> Option<&str> {
        self.send_error
            .as_deref()
            .or(self.connection_error.as_deref())
            .or(self.transcript_error.as_deref())
    }

    fn reconcile_user(&mut self, message: Message) {
        if let Some(turn) = self.turns.iter_mut().find(|turn| {
            let Delivery::Optimistic {
                submitted_at_ms, ..
            } = turn.delivery
            else {
                return false;
            };
            normalized_text(&turn.prompt.text) == normalized_text(&message.text)
                && turn.prompt.attachments.len() == message.attachments.len()
                && timestamps_match(
                    Some(submitted_at_ms),
                    message.timestamp_ms,
                    RECONCILE_WINDOW_MS,
                )
        }) {
            turn.prompt = message;
            turn.delivery = Delivery::Native;
        } else {
            let turn = Turn {
                prompt: message,
                final_answer: None,
                delivery: Delivery::Native,
            };
            if let Some(index) = self.replay_insert_at.as_mut() {
                self.turns.insert(*index, turn);
                *index += 1;
            } else {
                self.turns.push(turn);
            }
        }
    }
}

fn normalized_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_end()
        .to_owned()
}

fn timestamps_match(left: Option<u64>, right: Option<u64>, window_ms: u64) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.abs_diff(right) <= window_ms,
        _ => false,
    }
}
