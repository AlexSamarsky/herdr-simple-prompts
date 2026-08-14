use super::{AgentStatus, TranscriptAdapter};
use crate::model::ConversationEvent;
use crate::{AppError, AppResult};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const CHECKPOINT_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FollowerEvent {
    Conversation(ConversationEvent),
    Reloaded,
    ParseError { line: u64, message: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

pub struct TranscriptFollower {
    path: PathBuf,
    identity: FileIdentity,
    offset: u64,
    line_number: u64,
    partial: Vec<u8>,
    checkpoint: Vec<u8>,
    adapter: Box<dyn TranscriptAdapter>,
}

impl TranscriptFollower {
    pub fn new(path: impl AsRef<Path>, adapter: Box<dyn TranscriptAdapter>) -> AppResult<Self> {
        let path = path.as_ref().to_owned();
        let metadata = std::fs::metadata(&path).map_err(|error| {
            AppError::new(
                "transcript follower",
                format!("cannot inspect {}: {error}", path.display()),
            )
        })?;
        Ok(Self {
            path,
            identity: identity(&metadata),
            offset: 0,
            line_number: 0,
            partial: Vec::new(),
            checkpoint: Vec::new(),
            adapter,
        })
    }

    pub fn poll(&mut self) -> AppResult<Vec<FollowerEvent>> {
        let metadata = std::fs::metadata(&self.path)
            .map_err(|error| AppError::new("transcript follower", error.to_string()))?;
        let current_identity = identity(&metadata);
        let checkpoint_changed = current_identity == self.identity
            && metadata.len() >= self.offset
            && !self.checkpoint_matches()?;
        let reloaded =
            current_identity != self.identity || metadata.len() < self.offset || checkpoint_changed;
        if reloaded {
            self.identity = current_identity;
            self.offset = 0;
            self.line_number = 0;
            self.partial.clear();
            self.checkpoint.clear();
            self.adapter.reset();
        }

        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut appended = Vec::new();
        file.read_to_end(&mut appended)?;
        self.offset += appended.len() as u64;
        self.checkpoint.extend_from_slice(&appended);
        if self.checkpoint.len() > CHECKPOINT_BYTES {
            self.checkpoint
                .drain(..self.checkpoint.len() - CHECKPOINT_BYTES);
        }
        self.partial.extend_from_slice(&appended);

        let complete_end = self
            .partial
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let complete = self.partial.drain(..complete_end).collect::<Vec<_>>();
        let mut events = Vec::new();
        if reloaded {
            events.push(FollowerEvent::Reloaded);
        }
        for bytes in complete.split(|byte| *byte == b'\n') {
            if bytes.is_empty() {
                continue;
            }
            self.line_number += 1;
            let line = match std::str::from_utf8(bytes) {
                Ok(line) => line,
                Err(error) => {
                    events.push(FollowerEvent::ParseError {
                        line: self.line_number,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            match self.adapter.ingest_line(self.line_number, line) {
                Ok(conversation_events) => events.extend(
                    conversation_events
                        .into_iter()
                        .map(FollowerEvent::Conversation),
                ),
                Err(error) => events.push(FollowerEvent::ParseError {
                    line: self.line_number,
                    message: error.to_string(),
                }),
            }
        }
        Ok(events)
    }

    pub fn poll_initial(&mut self, status: AgentStatus) -> AppResult<Vec<FollowerEvent>> {
        self.poll_for_status(status)
    }

    pub fn poll_for_status(&mut self, status: AgentStatus) -> AppResult<Vec<FollowerEvent>> {
        let mut events = self.poll()?;
        if !status.keeps_turn_open() {
            if let Some(final_answer) = self.finalize_pending() {
                events.push(final_answer);
            }
        }
        Ok(events)
    }

    pub fn finalize_pending(&mut self) -> Option<FollowerEvent> {
        self.adapter
            .finalize_pending()
            .map(FollowerEvent::Conversation)
    }

    fn checkpoint_matches(&self) -> AppResult<bool> {
        if self.checkpoint.is_empty() {
            return Ok(true);
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(
            self.offset.saturating_sub(self.checkpoint.len() as u64),
        ))?;
        let mut current = vec![0; self.checkpoint.len()];
        file.read_exact(&mut current)?;
        Ok(current == self.checkpoint)
    }
}

fn identity(metadata: &std::fs::Metadata) -> FileIdentity {
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}
