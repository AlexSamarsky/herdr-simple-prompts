use crate::ansi::sanitize_ansi;
use crate::markdown::style_markdown;
use crate::model::{Attachment, Message};
use crate::paste::fingerprint;
use crate::state::{
    ensure_private_directory, nofollow_flag, reject_symlink, safe_state_component,
    validate_private_directory, validate_regular_file,
};
use crate::style::{MessagePresentation, StyleRun, StyledText, validate_styled_text};
use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const HISTORY_VERSION: u8 = 2;
const LEGACY_HISTORY_VERSION: u8 = 1;
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(2);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const WRITER_RETRY_BACKOFF: Duration = Duration::from_millis(100);

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_text_fingerprint: Option<u64>,
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
            None,
            None,
        );
        record.validate()?;
        Ok(record)
    }

    pub fn final_answer(
        message: &Message,
        turn_id: impl Into<String>,
        order: u64,
    ) -> AppResult<Self> {
        let (presentation, rendered_text, rendered_text_fingerprint) = match &message.presentation {
            MessagePresentation::NativeAnsi(styled) => (
                PersistedPresentation::NativeAnsi(styled.runs.clone()),
                Some(styled.text.clone()),
                Some(fingerprint(&styled.text)),
            ),
            MessagePresentation::MarkdownFallback => (PersistedPresentation::Fallback, None, None),
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
            rendered_text,
            rendered_text_fingerprint,
        );
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> AppResult<()> {
        if !matches!(self.version, LEGACY_HISTORY_VERSION | HISTORY_VERSION) {
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
            | (VisibleRole::Final, PersistedPresentation::Fallback) => {
                if self.rendered_text.is_some() || self.rendered_text_fingerprint.is_some() {
                    return Err(AppError::new(
                        "history journal",
                        "non-native presentation carries rendered text",
                    ));
                }
            }
            (VisibleRole::Final, PersistedPresentation::NativeAnsi(runs)) => {
                if self.version == LEGACY_HISTORY_VERSION {
                    if self.rendered_text.is_some() || self.rendered_text_fingerprint.is_some() {
                        return Err(AppError::new(
                            "history journal",
                            "legacy native presentation carries rendered text",
                        ));
                    }
                    validate_styled_text(&StyledText {
                        text: self.text.clone(),
                        runs: runs.clone(),
                    })
                    .map_err(|error| AppError::new("history journal", error))?;
                } else {
                    let (Some(rendered_text), Some(rendered_text_fingerprint)) =
                        (self.rendered_text.as_ref(), self.rendered_text_fingerprint)
                    else {
                        return Err(AppError::new(
                            "history journal",
                            "native presentation is missing rendered text",
                        ));
                    };
                    if rendered_text_fingerprint != fingerprint(rendered_text) {
                        return Err(AppError::new(
                            "history journal",
                            "rendered text fingerprint does not match",
                        ));
                    }
                    validate_styled_text(&StyledText {
                        text: rendered_text.clone(),
                        runs: runs.clone(),
                    })
                    .map_err(|error| AppError::new("history journal", error))?;
                }
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
            PersistedPresentation::NativeAnsi(runs) if self.version == HISTORY_VERSION => self
                .rendered_text
                .map(|text| MessagePresentation::NativeAnsi(StyledText { text, runs }))
                .unwrap_or(MessagePresentation::MarkdownFallback),
            PersistedPresentation::NativeAnsi(runs) => {
                let projected = style_markdown(&self.text);
                if projected.text == self.text {
                    MessagePresentation::NativeAnsi(StyledText {
                        text: self.text.clone(),
                        runs,
                    })
                } else {
                    MessagePresentation::MarkdownFallback
                }
            }
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
        rendered_text: Option<String>,
        rendered_text_fingerprint: Option<u64>,
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
            rendered_text,
            rendered_text_fingerprint,
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
        for directory in &self.directories {
            if !validate_private_directory(directory)? {
                return Ok(Vec::new());
            }
        }
        match std::fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        reject_symlink(&self.path)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(nofollow_flag())
            .open(&self.path)?;
        validate_regular_file(&file, &self.path)?;
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
        if self.append_cancellable(record, || false)? {
            Ok(())
        } else {
            Err(AppError::new(
                "history journal",
                "journal append was cancelled",
            ))
        }
    }

    fn append_cancellable(
        &self,
        record: &VisibleHistoryRecord,
        cancelled: impl Fn() -> bool,
    ) -> AppResult<bool> {
        record.validate()?;
        let mut bytes = serde_json::to_vec(record)?;
        bytes.push(b'\n');
        self.ensure_private_directories()?;
        reject_symlink(&self.path)?;
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .read(true)
            .mode(0o600)
            .custom_flags(nofollow_flag())
            .open(&self.path)?;
        validate_regular_file(&file, &self.path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        let Some(_lock) = FileLock::exclusive(&file, &cancelled)? else {
            return Ok(false);
        };
        truncate_incomplete_tail(&mut file)?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        Ok(true)
    }

    fn ensure_private_directories(&self) -> AppResult<()> {
        for directory in &self.directories {
            ensure_private_directory(directory)?;
        }
        Ok(())
    }
}

fn truncate_incomplete_tail(file: &mut File) -> AppResult<()> {
    let length = file.metadata()?.len();
    if length == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0_u8; 1];
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(());
    }

    let mut cursor = length;
    let mut buffer = [0_u8; 8 * 1024];
    while cursor > 0 {
        let chunk = usize::try_from(cursor.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let start = cursor - chunk as u64;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..chunk])?;
        if let Some(index) = buffer[..chunk].iter().rposition(|byte| *byte == b'\n') {
            file.set_len(start + index as u64 + 1)?;
            file.seek(SeekFrom::End(0))?;
            return Ok(());
        }
        cursor = start;
    }
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

struct FileLock {
    fd: std::os::raw::c_int,
}

impl FileLock {
    fn exclusive(file: &std::fs::File, cancelled: impl Fn() -> bool) -> AppResult<Option<Self>> {
        let fd = file.as_raw_fd();
        let deadline = Instant::now() + LOCK_WAIT_LIMIT;
        loop {
            if cancelled() {
                return Ok(None);
            }
            match flock(fd, LOCK_EX | LOCK_NB) {
                Ok(()) => return Ok(Some(Self { fd })),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(AppError::new(
                            "history journal",
                            "timed out waiting for the journal lock",
                        ));
                    }
                    thread::sleep(LOCK_RETRY_INTERVAL);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = flock(self.fd, LOCK_UN);
    }
}

const LOCK_EX: std::os::raw::c_int = 2;
const LOCK_UN: std::os::raw::c_int = 8;
const LOCK_NB: std::os::raw::c_int = 4;

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
    cancel: bool,
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
                    if state.cancel {
                        break;
                    }
                    if state.pending.is_empty() && state.shutdown {
                        break;
                    }
                    std::mem::take(&mut state.pending)
                };
                let mut records = pending.into_iter();
                while let Some((key, record)) = records.next() {
                    let append = journal.append_cancellable(&record, || {
                        worker_slot
                            .0
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .cancel
                    });
                    if matches!(append, Ok(false)) {
                        return;
                    }
                    if let Err(write_error) = append {
                        {
                            let mut first_error = worker_error
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            if first_error.is_none() {
                                *first_error = Some(write_error.to_string());
                            }
                        }
                        let (lock, ready) = &*worker_slot;
                        let mut state =
                            lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        if state.cancel || state.shutdown {
                            return;
                        }
                        state.pending.entry(key).or_insert(record);
                        for (remaining_key, remaining_record) in records {
                            state
                                .pending
                                .entry(remaining_key)
                                .or_insert(remaining_record);
                        }
                        let _ = ready
                            .wait_timeout(state, WRITER_RETRY_BACKOFF)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        break;
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

    pub fn cancel(mut self) {
        let (lock, ready) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending.clear();
        state.cancel = true;
        state.shutdown = true;
        ready.notify_one();
        drop(state);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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

#[cfg(test)]
mod tests {
    use super::{
        FileLock, HistoryJournal, HistoryWriter, LOCK_WAIT_LIMIT, PersistedPresentation,
        VisibleHistoryRecord, VisibleRole,
    };
    use crate::paste::fingerprint;
    use std::fs::OpenOptions;
    use std::time::{Duration, Instant};

    #[test]
    fn exclusive_lock_wait_is_bounded_and_cancellable() {
        let path = std::env::temp_dir().join(format!(
            "herdr-simple-prompts-lock-timeout-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let first_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let second_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let _first = FileLock::exclusive(&first_file, || false)
            .unwrap()
            .expect("first lock should be acquired");

        let started = Instant::now();
        assert!(FileLock::exclusive(&second_file, || false).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));

        let started = Instant::now();
        assert!(
            FileLock::exclusive(&second_file, || true)
                .unwrap()
                .is_none()
        );
        assert!(started.elapsed() < Duration::from_millis(100));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn writer_recovers_after_a_transient_lock_timeout() {
        let root = std::env::temp_dir().join(format!(
            "herdr-simple-prompts-lock-recovery-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let journal = HistoryJournal::at(&root, "w1:p1", "session-1").unwrap();
        let first = VisibleHistoryRecord {
            version: 2,
            role: VisibleRole::Prompt,
            stable_id: "u1".into(),
            turn_id: "u1".into(),
            order: 1,
            text: "first".into(),
            attachments: Vec::new(),
            timestamp_ms: Some(1),
            text_fingerprint: fingerprint("first"),
            presentation: PersistedPresentation::Plain,
            rendered_text: None,
            rendered_text_fingerprint: None,
        };
        journal.append(&first).unwrap();
        let locked_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(journal.path())
            .unwrap();
        let lock = FileLock::exclusive(&locked_file, || false)
            .unwrap()
            .expect("test lock should be acquired");
        let writer = HistoryWriter::spawn(journal.clone());
        writer.queue(first.clone());
        std::thread::sleep(LOCK_WAIT_LIMIT + Duration::from_millis(100));
        assert!(writer.take_error().is_some());

        drop(lock);
        let mut second = first.clone();
        second.stable_id = "u2".into();
        second.turn_id = "u2".into();
        second.order = 2;
        second.text = "second".into();
        second.text_fingerprint = fingerprint("second");
        writer.queue(second.clone());
        drop(writer);

        assert_eq!(journal.load().unwrap(), vec![first, second]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
