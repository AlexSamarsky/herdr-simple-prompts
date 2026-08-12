use crate::ansi::sanitize_ansi;
use crate::model::{Attachment, Message};
use crate::paste::fingerprint;
use crate::state::safe_state_component;
use crate::style::{MessagePresentation, StyleRun, validate_style_runs};
use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

const HISTORY_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibleRole {
    Prompt,
    Final,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct VisibleAttachment {
    pub id: String,
    pub display: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedPresentation {
    Plain,
    NativeAnsi(Vec<StyleRun>),
    Fallback,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct VisibleHistoryRecord {
    pub version: u8,
    pub role: VisibleRole,
    pub stable_id: String,
    pub turn_id: String,
    pub order: u64,
    pub text: String,
    pub attachments: Vec<VisibleAttachment>,
    pub timestamp_ms: Option<u64>,
    pub text_fingerprint: u64,
    pub presentation: PersistedPresentation,
}

impl VisibleHistoryRecord {
    pub fn prompt(message: &Message, order: u64) -> AppResult<Self> {
        if message.presentation != MessagePresentation::Plain {
            return Err(AppError::new(
                "history journal",
                "prompt presentation must be plain",
            ));
        }
        let record = Self::from_message(
            VisibleRole::Prompt,
            message.stable_id.clone(),
            message,
            order,
            PersistedPresentation::Plain,
        );
        record.validate()?;
        Ok(record)
    }

    pub fn final_answer(
        message: &Message,
        turn_id: impl Into<String>,
        order: u64,
    ) -> AppResult<Self> {
        let presentation = match &message.presentation {
            MessagePresentation::NativeAnsi(runs) => {
                PersistedPresentation::NativeAnsi(runs.clone())
            }
            MessagePresentation::MarkdownFallback => PersistedPresentation::Fallback,
            MessagePresentation::Plain => {
                return Err(AppError::new(
                    "history journal",
                    "final presentation cannot be plain",
                ));
            }
        };
        let record = Self::from_message(
            VisibleRole::Final,
            turn_id.into(),
            message,
            order,
            presentation,
        );
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.version != HISTORY_VERSION {
            return Err(AppError::new(
                "history journal",
                format!("unsupported record version {}", self.version),
            ));
        }
        if !valid_identifier(&self.stable_id) || !valid_identifier(&self.turn_id) {
            return Err(AppError::new(
                "history journal",
                "record identifiers must be non-empty printable text",
            ));
        }
        if self.text_fingerprint != fingerprint(&self.text) {
            return Err(AppError::new(
                "history journal",
                "record text fingerprint does not match",
            ));
        }
        match (&self.role, &self.presentation) {
            (VisibleRole::Prompt, PersistedPresentation::Plain)
            | (VisibleRole::Final, PersistedPresentation::Fallback) => {}
            (VisibleRole::Final, PersistedPresentation::NativeAnsi(runs)) => {
                validate_style_runs(&self.text, runs)
                    .map_err(|error| AppError::new("history journal", error))?;
            }
            (VisibleRole::Prompt, _) => {
                return Err(AppError::new(
                    "history journal",
                    "prompt carries answer-only presentation",
                ));
            }
            (VisibleRole::Final, PersistedPresentation::Plain) => {
                return Err(AppError::new(
                    "history journal",
                    "final presentation cannot be plain",
                ));
            }
        }
        if self.attachments.iter().any(|attachment| {
            !valid_identifier(&attachment.id)
                || attachment.display != sanitize_display_label(&attachment.display)
        }) {
            return Err(AppError::new(
                "history journal",
                "attachment metadata is not display-safe",
            ));
        }
        Ok(())
    }

    pub(crate) fn into_message(self) -> Message {
        let presentation = match self.presentation {
            PersistedPresentation::Plain => MessagePresentation::Plain,
            PersistedPresentation::NativeAnsi(runs) => MessagePresentation::NativeAnsi(runs),
            PersistedPresentation::Fallback => MessagePresentation::MarkdownFallback,
        };
        Message::restored(
            self.stable_id,
            self.text,
            presentation,
            self.attachments
                .into_iter()
                .map(|attachment| Attachment {
                    id: attachment.id,
                    display: attachment.display,
                    native_path: None,
                })
                .collect(),
            self.timestamp_ms,
        )
    }

    fn from_message(
        role: VisibleRole,
        turn_id: String,
        message: &Message,
        order: u64,
        presentation: PersistedPresentation,
    ) -> Self {
        Self {
            version: HISTORY_VERSION,
            role,
            stable_id: message.stable_id.clone(),
            turn_id,
            order,
            text: message.text.clone(),
            attachments: message
                .attachments
                .iter()
                .map(|attachment| VisibleAttachment {
                    id: attachment.id.clone(),
                    display: sanitize_display_label(&attachment.display),
                })
                .collect(),
            timestamp_ms: message.timestamp_ms,
            text_fingerprint: fingerprint(&message.text),
            presentation,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HistoryJournal {
    path: PathBuf,
    directories: [PathBuf; 3],
}

impl HistoryJournal {
    pub fn at(
        state_root: impl AsRef<Path>,
        source_pane: &str,
        session_id: &str,
    ) -> AppResult<Self> {
        let safe_pane = checked_safe_component(source_pane)?;
        let safe_session = checked_safe_component(session_id)?;
        let root = state_root.as_ref().to_owned();
        let history = root.join("history");
        let pane = history.join(safe_pane);
        let path = pane.join(format!("{safe_session}.jsonl"));
        Ok(Self {
            path,
            directories: [root, history, pane],
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> AppResult<Vec<VisibleHistoryRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = OpenOptions::new().read(true).open(&self.path)?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut line_number = 0_u64;
        let mut latest = BTreeMap::<(VisibleRole, String), (u64, VisibleHistoryRecord)>::new();
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            line_number = line_number.saturating_add(1);
            if line.last() != Some(&b'\n') {
                break;
            }
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let Ok(record) = serde_json::from_slice::<VisibleHistoryRecord>(&line) else {
                continue;
            };
            if record.validate().is_err() {
                continue;
            }
            latest.insert(
                (record.role, record.stable_id.clone()),
                (line_number, record),
            );
        }
        let mut records = latest.into_values().collect::<Vec<_>>();
        records.sort_by_key(|(line_number, record)| (record.order, *line_number));
        Ok(records.into_iter().map(|(_, record)| record).collect())
    }

    pub fn append(&self, record: &VisibleHistoryRecord) -> AppResult<()> {
        record.validate()?;
        let mut bytes = serde_json::to_vec(record)?;
        bytes.push(b'\n');
        self.ensure_private_directories()?;
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&self.path)?;
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        let _lock = FileLock::exclusive(&file)?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        Ok(())
    }

    fn ensure_private_directories(&self) -> AppResult<()> {
        for directory in &self.directories {
            std::fs::create_dir_all(directory)?;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
}

struct FileLock {
    fd: std::os::raw::c_int,
}

impl FileLock {
    fn exclusive(file: &std::fs::File) -> AppResult<Self> {
        let fd = file.as_raw_fd();
        flock(fd, LOCK_EX)?;
        Ok(Self { fd })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = flock(self.fd, LOCK_UN);
    }
}

const LOCK_EX: std::os::raw::c_int = 2;
const LOCK_UN: std::os::raw::c_int = 8;

fn flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::io::Result<()> {
    unsafe extern "C" {
        #[link_name = "flock"]
        fn os_flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int)
        -> std::os::raw::c_int;
    }

    if unsafe { os_flock(fd, operation) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[derive(Default)]
struct HistorySlot {
    pending: BTreeMap<(VisibleRole, String), VisibleHistoryRecord>,
    shutdown: bool,
}

pub struct HistoryWriter {
    slot: Arc<(Mutex<HistorySlot>, Condvar)>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}

impl HistoryWriter {
    pub fn spawn(journal: HistoryJournal) -> Self {
        let slot = Arc::new((Mutex::new(HistorySlot::default()), Condvar::new()));
        let error = Arc::new(Mutex::new(None));
        let worker_slot = Arc::clone(&slot);
        let worker_error = Arc::clone(&error);
        let worker = thread::spawn(move || {
            loop {
                let pending = {
                    let (lock, ready) = &*worker_slot;
                    let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    while state.pending.is_empty() && !state.shutdown {
                        state = ready
                            .wait(state)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                    if state.pending.is_empty() && state.shutdown {
                        break;
                    }
                    std::mem::take(&mut state.pending)
                };
                for record in pending.into_values() {
                    if let Err(write_error) = journal.append(&record) {
                        let mut first_error = worker_error
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if first_error.is_none() {
                            *first_error = Some(write_error.to_string());
                        }
                    }
                }
            }
        });
        Self {
            slot,
            error,
            worker: Some(worker),
        }
    }

    pub fn queue(&self, record: VisibleHistoryRecord) {
        let key = (record.role, record.stable_id.clone());
        let (lock, ready) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending.insert(key, record);
        ready.notify_one();
    }

    pub fn take_error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

impl Drop for HistoryWriter {
    fn drop(&mut self) {
        let (lock, ready) = &*self.slot;
        lock.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shutdown = true;
        ready.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn checked_safe_component(value: &str) -> AppResult<String> {
    let safe = safe_state_component(value);
    if safe.is_empty() {
        return Err(AppError::new(
            "history journal",
            "pane and session identifiers must not be empty",
        ));
    }
    Ok(safe)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| !character.is_control())
}

fn sanitize_display_label(display: &str) -> String {
    sanitize_ansi(display)
        .text
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}
