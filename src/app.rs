use crate::agent::AgentStatus;
use crate::composer::{ComposerAccess, NativeComposerState};
use crate::editor::{EditorSnapshot, EditorSubmission};
use crate::history::{VisibleHistoryRecord, VisibleRole};
use crate::model::{Attachment, Delivery, Message, Turn};
use crate::paste::fingerprint;
use crate::paste::{CompactPromptOverride, canonicalize_compact_markers, marker_counts};
use crate::status::StatusLine;
use crate::style::{MessagePresentation, StyledText, validate_styled_text};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

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
    FinalPresentation {
        stable_id: String,
        text_fingerprint: u64,
        presentation: MessagePresentation,
    },
}

/// Something the overlay has asked the pane to do and is waiting on.
///
/// Attaching or removing an image is not instant — the agent has to take the
/// picture out of the clipboard, or redraw without it — and a composer that
/// simply sits there for a few seconds looks broken. Saying what is being
/// waited for, and for how long, is the difference between waiting and
/// wondering.
#[derive(Clone, Debug)]
pub struct PendingAction {
    pub label: &'static str,
    pub since: Instant,
}

impl PendingAction {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            since: Instant::now(),
        }
    }
}

/// How long a notice about something that happened stays on screen: long
/// enough to read the line, short enough that it does not outlive what it is
/// about.
pub const NOTICE_LINGER: Duration = Duration::from_secs(12);

/// Which of the error lines is the one being shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Notice {
    Send,
    Connection,
    Transcript,
    Background,
}

pub struct AppState {
    pub session_id: String,
    pub turns: Vec<Turn>,
    pub draft: EditorSnapshot,
    pub draft_attachments: Vec<Attachment>,
    pub pending_attachments: Vec<Attachment>,
    pub native_composer: NativeComposerState,
    pub prompt_displays: Vec<CompactPromptOverride>,
    pub agent_status: AgentStatus,
    pub working_since: Option<Instant>,
    pub pending_action: Option<PendingAction>,
    pub status_line: Option<StatusLine>,
    pub connection_error: Option<String>,
    pub transcript_error: Option<String>,
    pub send_error: Option<String>,
    /// Draft and history writer failures. They are not send failures and must
    /// not masquerade as one, or the user retries a prompt that was delivered.
    pub background_error: Option<String>,
    /// What the error line is showing and since when, so a notice about
    /// something that happened can leave the screen again.
    pub notice_shown: Option<(String, Instant)>,
    pub blocked_surface: Option<Result<StyledText, String>>,
    pub interaction_error: Option<String>,
    pub source_pane_closed: bool,
    pub input_enabled: bool,
    pub scroll_from_bottom: usize,
    #[doc(hidden)]
    pub replay_insert_at: Option<usize>,
    #[doc(hidden)]
    pub history_orders: BTreeMap<(VisibleRole, String), u64>,
    #[doc(hidden)]
    pub next_history_order: u64,
    #[doc(hidden)]
    pub pending_history_upserts: Vec<VisibleHistoryRecord>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            turns: Vec::new(),
            draft: EditorSnapshot::default(),
            draft_attachments: Vec::new(),
            pending_attachments: Vec::new(),
            native_composer: NativeComposerState::Clear,
            prompt_displays: Vec::new(),
            agent_status: AgentStatus::Unknown,
            working_since: None,
            pending_action: None,
            status_line: None,
            connection_error: None,
            transcript_error: None,
            send_error: None,
            background_error: None,
            notice_shown: None,
            blocked_surface: None,
            interaction_error: None,
            source_pane_closed: false,
            input_enabled: true,
            scroll_from_bottom: 0,
            replay_insert_at: None,
            history_orders: BTreeMap::new(),
            next_history_order: 1,
            pending_history_upserts: Vec::new(),
        }
    }
}

impl AppState {
    pub fn composer_access(&self) -> ComposerAccess {
        let confirmed = self.draft_attachments.len();
        let pending = self.pending_attachments.len();
        if pending > 0 {
            return match self.native_composer {
                NativeComposerState::Clear if confirmed == 0 => ComposerAccess::Ready,
                NativeComposerState::OwnedAttachments(actual)
                    if actual == confirmed.saturating_add(pending) =>
                {
                    ComposerAccess::Ready
                }
                NativeComposerState::Unknown => ComposerAccess::Unknown,
                NativeComposerState::Clear
                | NativeComposerState::OwnedAttachments(_)
                | NativeComposerState::Occupied => ComposerAccess::Occupied,
            };
        }
        self.native_composer.access(confirmed)
    }

    pub fn source_pane_closed(&mut self) {
        self.source_pane_closed = true;
        self.agent_status = AgentStatus::Unknown;
        self.working_since = None;
        self.input_enabled = false;
        self.native_composer = NativeComposerState::Unknown;
        self.blocked_surface = None;
        self.interaction_error = None;
        self.connection_error = Some("Source pane closed · prefix+m to return".to_owned());
    }

    pub fn update_blocked_surface(
        &mut self,
        status: AgentStatus,
        surface: Option<Result<StyledText, String>>,
    ) {
        self.agent_status = status;
        if status == AgentStatus::Blocked {
            self.blocked_surface = surface;
        } else {
            self.blocked_surface = None;
            self.interaction_error = None;
        }
    }

    pub fn apply_interaction_result(&mut self, result: Result<(), String>) {
        if self.agent_status != AgentStatus::Blocked {
            return;
        }
        match result {
            Ok(()) => self.interaction_error = None,
            Err(error) => self.interaction_error = Some(error),
        }
    }

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
            AppEvent::NativeUser(message) => {
                let index = self.reconcile_user(message);
                self.queue_prompt_upsert(index);
            }
            AppEvent::NativeFinal(mut message) => {
                if message.presentation == MessagePresentation::Plain
                    || matches!(
                        &message.presentation,
                        MessagePresentation::NativeAnsi(styled)
                            if validate_styled_text(styled).is_err()
                    )
                {
                    message.presentation = MessagePresentation::MarkdownFallback;
                }
                if let Some(index) = self.reconcile_final(message) {
                    self.queue_final_upsert(index);
                }
            }
            AppEvent::TranscriptReloaded => {
                self.replay_insert_at = Some(0);
            }
            AppEvent::TranscriptReplayComplete => self.replay_insert_at = None,
            AppEvent::SendFailed { local_id, reason } => {
                let target = self.turns.iter().position(|turn| {
                    matches!(
                        &turn.delivery,
                        Delivery::Optimistic { local_id: candidate, .. } if candidate == &local_id
                    )
                });
                if let Some(index) = target {
                    let previous = std::mem::replace(
                        &mut self.turns[index].delivery,
                        Delivery::Failed { reason },
                    );
                    match previous {
                        Delivery::Optimistic { recovery, .. } => {
                            self.draft = recovery;
                            self.draft_attachments
                                .clone_from(&self.turns[index].prompt.attachments);
                        }
                        other => self.turns[index].delivery = other,
                    }
                }
            }
            AppEvent::FinalPresentation {
                stable_id,
                text_fingerprint,
                presentation,
            } => {
                let target = self.turns.iter().enumerate().find_map(|(index, turn)| {
                    turn.final_answer
                        .as_ref()
                        .filter(|message| {
                            message.stable_id == stable_id
                                && fingerprint(&message.text) == text_fingerprint
                        })
                        .map(|_| index)
                });
                if let Some(index) = target {
                    let message = self.turns[index]
                        .final_answer
                        .as_mut()
                        .expect("target contains a final answer");
                    let valid = match &presentation {
                        MessagePresentation::NativeAnsi(styled) => {
                            validate_styled_text(styled).is_ok()
                        }
                        MessagePresentation::MarkdownFallback => {
                            !matches!(message.presentation, MessagePresentation::NativeAnsi(_))
                        }
                        MessagePresentation::Plain => false,
                    };
                    if valid {
                        message.presentation = presentation;
                        self.queue_final_upsert(index);
                    }
                }
            }
        }
    }

    pub fn hydrate_visible_history(&mut self, mut records: Vec<VisibleHistoryRecord>) {
        records.retain(|record| record.validate().is_ok());
        records.sort_by_key(|record| record.order);
        self.turns.clear();
        self.history_orders.clear();
        self.pending_history_upserts.clear();
        self.replay_insert_at = None;

        let mut final_records = Vec::new();
        let mut maximum_order = 0;
        for record in records {
            maximum_order = maximum_order.max(record.order);
            self.history_orders
                .insert((record.role, record.stable_id.clone()), record.order);
            match record.role {
                VisibleRole::Prompt => self.turns.push(Turn {
                    prompt: record.into_message(),
                    final_answer: None,
                    delivery: Delivery::Native,
                }),
                VisibleRole::Final => final_records.push(record),
            }
        }
        for record in final_records {
            let turn_id = record.turn_id.clone();
            if let Some(turn) = self
                .turns
                .iter_mut()
                .find(|turn| turn.prompt.stable_id == turn_id)
            {
                turn.final_answer = Some(record.into_message());
            }
        }
        self.next_history_order = maximum_order.saturating_add(1).max(1);
    }

    pub fn drain_history_upserts(&mut self) -> Vec<VisibleHistoryRecord> {
        std::mem::take(&mut self.pending_history_upserts)
    }

    pub fn visible_error(&self) -> Option<&str> {
        match self.visible_notice()? {
            Notice::Send => self.send_error.as_deref(),
            Notice::Connection => self.connection_error.as_deref(),
            Notice::Transcript => self.transcript_error.as_deref(),
            Notice::Background => self.background_error.as_deref(),
        }
    }

    fn visible_notice(&self) -> Option<Notice> {
        if self.send_error.is_some() {
            Some(Notice::Send)
        } else if self.connection_error.is_some() {
            Some(Notice::Connection)
        } else if self.transcript_error.is_some() {
            Some(Notice::Transcript)
        } else if self.background_error.is_some() {
            Some(Notice::Background)
        } else {
            None
        }
    }

    /// Lets a notice leave the screen once it has had its time there.
    ///
    /// These lines are about a moment that has passed: an image that would not
    /// go, a draft that could not be written. Left standing, they say the
    /// overlay is still in trouble long after it is not — and the next thing
    /// the user does is work around a problem that was over minutes ago.
    ///
    /// A lost connection is not a moment but a state, so it is left alone: it
    /// goes when it is mended, not when it has been read.
    pub fn expire_notice(&mut self) {
        let Some(notice) = self.visible_notice() else {
            self.notice_shown = None;
            return;
        };
        if matches!(notice, Notice::Connection) {
            self.notice_shown = None;
            return;
        }
        let showing = self.visible_error().unwrap_or_default().to_owned();
        match &self.notice_shown {
            Some((shown, since)) if *shown == showing => {
                if since.elapsed() < NOTICE_LINGER {
                    return;
                }
                match notice {
                    Notice::Send => self.send_error = None,
                    Notice::Transcript => self.transcript_error = None,
                    Notice::Background => self.background_error = None,
                    Notice::Connection => {}
                }
                self.notice_shown = None;
            }
            _ => self.notice_shown = Some((showing, Instant::now())),
        }
    }

    fn reconcile_user(&mut self, mut message: Message) -> usize {
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
            turn.delivery == Delivery::Native && turn.prompt.stable_id == message.stable_id
        }) {
            let mut turn = self.turns.remove(index);
            if !marker_counts(&turn.prompt.text).is_empty() {
                message.text.clone_from(&turn.prompt.text);
            }
            turn.prompt = message;
            return self.place_replayed_turn(turn, index);
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
            let previous = std::mem::replace(&mut self.turns[index].delivery, Delivery::Native);
            let (complete_text, paste_ranges) = match previous {
                Delivery::Optimistic {
                    complete_text,
                    paste_ranges,
                    ..
                } => (complete_text, paste_ranges),
                other => {
                    self.turns[index].delivery = other;
                    return index;
                }
            };
            let turn = &mut self.turns[index];
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
            if self.replay_insert_at.is_some() {
                let turn = self.turns.remove(index);
                self.place_replayed_turn(turn, index)
            } else {
                index
            }
        } else {
            let turn = Turn {
                prompt: message,
                final_answer: None,
                delivery: Delivery::Native,
            };
            if let Some(index) = self.replay_insert_at.as_mut() {
                let inserted = *index;
                self.turns.insert(inserted, turn);
                *index += 1;
                inserted
            } else {
                self.turns.push(turn);
                self.turns.len() - 1
            }
        }
    }

    fn reconcile_final(&mut self, mut message: Message) -> Option<usize> {
        let existing = self
            .turns
            .iter()
            .enumerate()
            .find_map(|(turn_index, turn)| {
                turn.final_answer
                    .as_ref()
                    .filter(|final_answer| final_answer.stable_id == message.stable_id)
                    .map(|_| turn_index)
            });
        if let Some(index) = existing {
            let saved = self.turns[index]
                .final_answer
                .take()
                .expect("existing final answer was located");
            if fingerprint(&saved.text) == fingerprint(&message.text)
                && matches!(saved.presentation, MessagePresentation::NativeAnsi(_))
            {
                message.presentation = saved.presentation;
            }
            self.turns[index].final_answer = Some(message);
            return Some(index);
        }

        let replay_target = self
            .replay_insert_at
            .and_then(|insert_at| insert_at.checked_sub(1))
            .filter(|index| {
                self.turns.get(*index).is_some_and(|turn| {
                    turn.delivery == Delivery::Native && turn.final_answer.is_none()
                })
            });
        let target = replay_target.or_else(|| {
            self.turns
                .iter()
                .rposition(|turn| turn.delivery == Delivery::Native && turn.final_answer.is_none())
        });
        if let Some(index) = target {
            self.turns[index].final_answer = Some(message);
            Some(index)
        } else {
            None
        }
    }

    fn place_replayed_turn(&mut self, turn: Turn, original_index: usize) -> usize {
        if let Some(insert_at) = self.replay_insert_at.as_mut() {
            let target = if original_index < *insert_at {
                insert_at.saturating_sub(1)
            } else {
                *insert_at
            };
            self.turns.insert(target, turn);
            *insert_at = target.saturating_add(1);
            target
        } else {
            self.turns.insert(original_index, turn);
            original_index
        }
    }

    fn queue_prompt_upsert(&mut self, turn_index: usize) {
        let message = self.turns[turn_index].prompt.clone();
        let order = self.order_for(VisibleRole::Prompt, &message.stable_id);
        if let Ok(record) = VisibleHistoryRecord::prompt(&message, order) {
            self.pending_history_upserts.push(record);
        }
    }

    fn queue_final_upsert(&mut self, turn_index: usize) {
        let turn_id = self.turns[turn_index].prompt.stable_id.clone();
        let Some(message) = self.turns[turn_index].final_answer.clone() else {
            return;
        };
        let order = self.order_for(VisibleRole::Final, &message.stable_id);
        if let Ok(record) = VisibleHistoryRecord::final_answer(&message, turn_id, order) {
            self.pending_history_upserts.push(record);
        }
    }

    fn order_for(&mut self, role: VisibleRole, stable_id: &str) -> u64 {
        let key = (role, stable_id.to_owned());
        if let Some(order) = self.history_orders.get(&key) {
            return *order;
        }
        let order = self.next_history_order;
        self.next_history_order = self.next_history_order.saturating_add(1);
        self.history_orders.insert(key, order);
        order
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
