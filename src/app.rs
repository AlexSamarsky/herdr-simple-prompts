use crate::agent::AgentStatus;
use crate::editor::{EditorSnapshot, EditorSubmission};
use crate::model::{Attachment, Delivery, Message, Turn};
use crate::paste::{CompactPromptOverride, canonicalize_compact_markers};
use crate::status::StatusLine;
use crate::style::MessagePresentation;
use std::time::Instant;

const RECONCILE_WINDOW_MS: u64 = 30_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    PromptSubmitted {
        local_id: String,
        submission: EditorSubmission,
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
    pub session_id: String,
    pub turns: Vec<Turn>,
    pub draft: EditorSnapshot,
    pub draft_attachments: Vec<Attachment>,
    pub pending_attachments: Vec<Attachment>,
    pub prompt_displays: Vec<CompactPromptOverride>,
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
            session_id: String::new(),
            turns: Vec::new(),
            draft: EditorSnapshot::default(),
            draft_attachments: Vec::new(),
            pending_attachments: Vec::new(),
            prompt_displays: Vec::new(),
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
                submission,
                attachments,
                at_ms,
            } => {
                self.draft = EditorSnapshot::default();
                self.draft_attachments.clear();
                let EditorSubmission {
                    complete_text,
                    display_text,
                    recovery,
                    paste_ranges,
                } = submission;
                self.turns.push(Turn {
                    prompt: Message {
                        stable_id: local_id.clone(),
                        text: display_text,
                        presentation: MessagePresentation::Plain,
                        attachments,
                        timestamp_ms: Some(at_ms),
                    },
                    final_answer: None,
                    delivery: Delivery::Optimistic {
                        local_id,
                        submitted_at_ms: at_ms,
                        complete_text,
                        recovery,
                        paste_ranges,
                    },
                });
            }
            AppEvent::NativeUser(message) => self.reconcile_user(message),
            AppEvent::NativeFinal(mut message) => {
                if message.presentation == MessagePresentation::Plain {
                    message.presentation = MessagePresentation::MarkdownFallback;
                }
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
                    let Delivery::Optimistic { recovery, .. } = &turn.delivery else {
                        unreachable!("matched only optimistic turns");
                    };
                    self.draft.clone_from(recovery);
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

    fn reconcile_user(&mut self, mut message: Message) {
        let provider_text = message.text.clone();
        if let Some(compact_text) = self
            .prompt_displays
            .iter()
            .find(|summary| {
                summary.session_id == self.session_id && summary.stable_id == message.stable_id
            })
            .and_then(|summary| summary.compact_text(&provider_text))
        {
            message.text = compact_text;
        }

        if let Some(index) = self.turns.iter().position(|turn| {
            let Delivery::Optimistic {
                submitted_at_ms,
                complete_text,
                paste_ranges,
                ..
            } = &turn.delivery
            else {
                return false;
            };
            (normalized_text(complete_text) == normalized_text(&provider_text)
                || normalized_text(&turn.prompt.text) == normalized_text(&provider_text)
                || (!paste_ranges.is_empty()
                    && normalized_text(&canonicalize_compact_markers(&provider_text))
                        == normalized_text(&turn.prompt.text)))
                && turn.prompt.attachments.len() == message.attachments.len()
                && timestamps_match(
                    Some(*submitted_at_ms),
                    message.timestamp_ms,
                    RECONCILE_WINDOW_MS,
                )
        }) {
            let turn = &mut self.turns[index];
            let Delivery::Optimistic {
                complete_text,
                paste_ranges,
                ..
            } = &turn.delivery
            else {
                unreachable!("matched only optimistic turns");
            };
            let complete_text = complete_text.clone();
            let paste_ranges = paste_ranges.clone();
            let display_text = if normalized_text(&provider_text) == normalized_text(&complete_text)
            {
                turn.prompt.text.clone()
            } else {
                message.text.clone()
            };
            let native_id = message.stable_id.clone();
            turn.prompt = Message {
                stable_id: native_id.clone(),
                text: display_text,
                presentation: MessagePresentation::Plain,
                attachments: message.attachments,
                timestamp_ms: message.timestamp_ms,
            };
            turn.delivery = Delivery::Native;

            if paste_ranges.is_empty() {
                self.prompt_displays.retain(|existing| {
                    existing.session_id != self.session_id || existing.stable_id != native_id
                });
            } else {
                let summary = CompactPromptOverride::new(
                    self.session_id.clone(),
                    native_id.clone(),
                    &complete_text,
                    paste_ranges,
                );
                if let Some(existing) = self.prompt_displays.iter_mut().find(|existing| {
                    existing.session_id == self.session_id && existing.stable_id == native_id
                }) {
                    *existing = summary;
                } else {
                    self.prompt_displays.push(summary);
                }
            }
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
