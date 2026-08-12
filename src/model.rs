use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub display: String,
    pub native_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub stable_id: String,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub timestamp_ms: Option<u64>,
}

impl Message {
    pub fn text(id: impl Into<String>, text: impl Into<String>, timestamp_ms: Option<u64>) -> Self {
        Self {
            stable_id: id.into(),
            text: text.into(),
            attachments: Vec::new(),
            timestamp_ms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Delivery {
    Native,
    Optimistic {
        local_id: String,
        submitted_at_ms: u64,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Turn {
    pub prompt: Message,
    pub final_answer: Option<Message>,
    pub delivery: Delivery,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationEvent {
    User(Message),
    Final(Message),
}
