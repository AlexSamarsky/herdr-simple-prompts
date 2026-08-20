use crate::editor::{EditorChunk, EditorSnapshot};
use crate::herdr::HerdrClient;
use crate::history::HistoryJournal;
use crate::model::Attachment;
use crate::paste::CompactPromptOverride;
use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PANE_NAMESPACE_VERSION: u8 = 1;
const DRAFT_VERSION: u8 = 4;
const ORPHAN_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const LIFECYCLE_LOCK_WAIT_LIMIT: Duration = Duration::from_secs(2);
const LIFECYCLE_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const STATE_LOCK_EX: std::os::raw::c_int = 2;
const STATE_LOCK_UN: std::os::raw::c_int = 8;
const STATE_LOCK_NB: std::os::raw::c_int = 4;
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[cfg(target_os = "linux")]
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(target_os = "macos")]
const O_NOFOLLOW: i32 = 0x0000_0100;
#[cfg(target_os = "linux")]
const O_DIRECTORY: i32 = 0o200000;
#[cfg(target_os = "macos")]
const O_DIRECTORY: i32 = 0x0010_0000;

#[derive(Default, Deserialize, Serialize)]
struct OverlayRegistry {
    overlays: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DraftState {
    pub session_id: Option<String>,
    pub text: String,
    pub editor: EditorSnapshot,
    pub attachments: Vec<Attachment>,
    pub prompt_displays: Vec<CompactPromptOverride>,
}

#[derive(Deserialize, Serialize)]
struct PersistedDraft {
    version: u8,
    #[serde(default)]
    session_id: Option<String>,
    editor: EditorSnapshot,
    attachments: Vec<PersistedAttachment>,
    #[serde(default)]
    prompt_displays: Vec<CompactPromptOverride>,
}

#[derive(Deserialize)]
struct LegacyDraft {
    text: String,
    attachments: Vec<PersistedAttachment>,
}

enum ReadDraft {
    Current(PersistedDraft),
    Legacy(LegacyDraft),
}

#[derive(Deserialize, Serialize)]
struct PersistedAttachment {
    id: String,
    display: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneNamespaceState {
    version: u8,
    source_pane: String,
    session_id: String,
    last_verified_ms: u64,
    orphaned_since_ms: Option<u64>,
}

#[derive(Clone)]
pub struct StateStore {
    root: PathBuf,
}

struct LifecycleLock {
    file: File,
}

impl LifecycleLock {
    fn acquire(root: &Path) -> AppResult<Self> {
        ensure_private_directory(root)?;
        let path = root.join(".lifecycle.lock");
        require_exact_parent(&path, root)?;
        reject_symlink(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(O_NOFOLLOW)
            .open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;

        let deadline = Instant::now() + LIFECYCLE_LOCK_WAIT_LIMIT;
        loop {
            match state_flock(file.as_raw_fd(), STATE_LOCK_EX | STATE_LOCK_NB) {
                Ok(()) => return Ok(Self { file }),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(AppError::new(
                            "plugin state",
                            "timed out waiting for the lifecycle lock",
                        ));
                    }
                    thread::sleep(LIFECYCLE_LOCK_RETRY_INTERVAL);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for LifecycleLock {
    fn drop(&mut self) {
        let _ = state_flock(self.file.as_raw_fd(), STATE_LOCK_UN);
    }
}

fn state_flock(fd: std::os::raw::c_int, operation: std::os::raw::c_int) -> std::io::Result<()> {
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

struct DraftSnapshot {
    editor: EditorSnapshot,
    prompt_displays: Vec<CompactPromptOverride>,
}

#[derive(Default)]
struct DraftSlot {
    pending: Option<DraftSnapshot>,
    shutdown: bool,
}

pub struct DraftWriter {
    slot: Arc<(Mutex<DraftSlot>, Condvar)>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}

impl DraftWriter {
    pub fn spawn(store: StateStore, pane_id: String, session_id: Option<String>) -> Self {
        let slot = Arc::new((Mutex::new(DraftSlot::default()), Condvar::new()));
        let error = Arc::new(Mutex::new(None));
        let worker_slot = Arc::clone(&slot);
        let worker_error = Arc::clone(&error);
        let worker = thread::spawn(move || {
            loop {
                let snapshot = {
                    let (lock, ready) = &*worker_slot;
                    let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    while state.pending.is_none() && !state.shutdown {
                        state = ready
                            .wait(state)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                    match state.pending.take() {
                        Some(snapshot) => snapshot,
                        None if state.shutdown => break,
                        None => continue,
                    }
                };
                if let Err(save_error) = store.save_editor_draft(
                    &pane_id,
                    session_id.as_deref(),
                    &snapshot.editor,
                    &snapshot.prompt_displays,
                ) {
                    *worker_error
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(save_error.to_string());
                }
            }
        });
        Self {
            slot,
            error,
            worker: Some(worker),
        }
    }

    pub fn queue_editor(
        &self,
        editor: EditorSnapshot,
        prompt_displays: Vec<CompactPromptOverride>,
    ) {
        let (lock, ready) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending = Some(DraftSnapshot {
            editor,
            prompt_displays,
        });
        ready.notify_one();
    }

    pub fn queue(&self, text: String, attachments: Vec<Attachment>) {
        let mut editor = EditorSnapshot {
            chunks: attachments
                .into_iter()
                .map(|attachment| EditorChunk::Attachment {
                    id: attachment.id,
                    display: attachment.display,
                })
                .collect(),
        };
        editor.chunks.extend(EditorSnapshot::plain(text).chunks);
        self.queue_editor(editor, Vec::new());
    }

    pub fn take_error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

impl Drop for DraftWriter {
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

impl StateStore {
    pub fn at(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_owned(),
        }
    }

    pub fn with_lifecycle_lock<T>(&self, operation: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
        let _lock = LifecycleLock::acquire(&self.root)?;
        operation()
    }

    pub fn save_overlay(&self, source: &str, overlay: &str) -> AppResult<()> {
        let mut registry = self.load_registry()?;
        registry
            .overlays
            .insert(source.to_owned(), overlay.to_owned());
        self.save_registry(&registry)
    }

    pub fn history_journal(
        &self,
        source_pane: &str,
        session_id: &str,
    ) -> AppResult<HistoryJournal> {
        HistoryJournal::at(&self.root, source_pane, session_id)
    }

    pub fn remove_source(&self, source: &str) -> AppResult<()> {
        let mut registry = self.load_registry()?;
        registry.overlays.remove(source);
        self.save_registry(&registry)
    }

    pub fn remove_pane_state(&self, source_pane: &str) -> AppResult<()> {
        if !validate_private_directory(&self.root)? {
            return Ok(());
        }
        let safe = checked_pane_component(source_pane)?;
        let draft = self.root.join(format!("draft-{safe}.json"));
        let history_root = self.root.join("history");
        let history = history_root.join(&safe);
        let panes_root = self.root.join("panes");
        let namespace = panes_root.join(format!("{safe}.json"));

        require_exact_parent(&draft, &self.root)?;
        require_exact_parent(&history, &history_root)?;
        require_exact_parent(&namespace, &panes_root)?;
        reject_symlink(&history_root)?;
        reject_symlink(&panes_root)?;
        reject_symlink(&draft)?;
        reject_symlink(&history)?;
        reject_symlink(&namespace)?;

        remove_file_if_present(&draft)?;
        remove_directory_if_present(&history)?;
        self.remove_source(source_pane)?;
        remove_file_if_present(&namespace)?;
        Ok(())
    }

    pub fn bind_verified_namespace(
        &self,
        source_pane: &str,
        session_id: &str,
        now_ms: u64,
    ) -> AppResult<()> {
        let safe = checked_pane_component(source_pane)?;
        if session_id.is_empty() || session_id.chars().any(char::is_control) {
            return Err(AppError::new(
                "plugin state",
                "session id must be non-empty printable text",
            ));
        }
        let panes = self.root.join("panes");
        ensure_private_directory(&self.root)?;
        ensure_private_directory(&panes)?;
        let destination = panes.join(format!("{safe}.json"));
        require_exact_parent(&destination, &panes)?;
        reject_symlink(&destination)?;
        self.bind_draft_after_verification(source_pane, session_id)?;
        atomic_write(
            &self.root,
            &destination,
            serde_json::to_vec(&PaneNamespaceState {
                version: PANE_NAMESPACE_VERSION,
                source_pane: source_pane.to_owned(),
                session_id: session_id.to_owned(),
                last_verified_ms: now_ms,
                orphaned_since_ms: None,
            })?,
        )
    }

    pub fn validate_saved_namespaces(&self, client: &HerdrClient, now_ms: u64) -> AppResult<()> {
        if !validate_private_directory(&self.root)? {
            return Ok(());
        }
        let panes = self.root.join("panes");
        if !panes.exists() {
            return Ok(());
        }
        reject_symlink(&panes)?;
        let mut paths = std::fs::read_dir(&panes)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            require_exact_parent(&path, &panes)?;
            reject_symlink(&path)?;
            let mut namespace: PaneNamespaceState = serde_json::from_slice(&std::fs::read(&path)?)?;
            let safe = checked_pane_component(&namespace.source_pane)?;
            if namespace.version != PANE_NAMESPACE_VERSION
                || path.file_name().and_then(|value| value.to_str())
                    != Some(&format!("{safe}.json"))
            {
                return Err(AppError::new(
                    "plugin state",
                    format!("invalid pane namespace metadata: {}", path.display()),
                ));
            }

            match client.agent_get(&namespace.source_pane) {
                Ok(result) => match verified_session(&result, &namespace.source_pane) {
                    Ok(live_session) if live_session == namespace.session_id => {
                        self.bind_draft_after_verification(&namespace.source_pane, &live_session)?;
                        namespace.last_verified_ms = now_ms;
                        namespace.orphaned_since_ms = None;
                        self.save_namespace(&namespace)?;
                    }
                    Ok(live_session) => {
                        self.remove_replaced_session_state(&namespace, &live_session)?;
                    }
                    Err(_) => self.retain_or_expire_orphan(namespace, now_ms)?,
                },
                Err(error) if error.is_pane_not_found() || error.is_agent_not_found() => {
                    self.remove_pane_state(&namespace.source_pane)?;
                }
                Err(_) => self.retain_or_expire_orphan(namespace, now_ms)?,
            }
        }
        Ok(())
    }

    pub fn overlay_for_source(&self, source: &str) -> AppResult<Option<String>> {
        Ok(self.load_registry()?.overlays.get(source).cloned())
    }

    pub fn source_for_overlay(&self, overlay: &str) -> AppResult<Option<String>> {
        Ok(self
            .load_registry()?
            .overlays
            .into_iter()
            .find_map(|(source, candidate)| (candidate == overlay).then_some(source)))
    }

    pub fn save_draft(
        &self,
        pane_id: &str,
        text: &str,
        attachments: &[Attachment],
    ) -> AppResult<()> {
        let mut snapshot = EditorSnapshot {
            chunks: attachments
                .iter()
                .map(|attachment| EditorChunk::Attachment {
                    id: attachment.id.clone(),
                    display: attachment.display.clone(),
                })
                .collect(),
        };
        snapshot.chunks.extend(EditorSnapshot::plain(text).chunks);
        self.save_editor_draft(pane_id, None, &snapshot, &[])
    }

    /// Attachments are part of the editor snapshot: they hold a place in the
    /// line, so they are saved where they sit rather than in a list beside it.
    pub fn save_editor_draft(
        &self,
        pane_id: &str,
        session_id: Option<&str>,
        editor: &EditorSnapshot,
        prompt_displays: &[CompactPromptOverride],
    ) -> AppResult<()> {
        let file = self
            .root
            .join(format!("draft-{}.json", safe_state_component(pane_id)));
        atomic_write(
            &self.root,
            &file,
            serde_json::to_vec(&PersistedDraft {
                version: DRAFT_VERSION,
                session_id: session_id.map(str::to_owned),
                editor: editor.clone(),
                attachments: Vec::new(),
                prompt_displays: prompt_displays.to_vec(),
            })?,
        )
    }

    pub fn load_draft(&self, pane_id: &str) -> AppResult<DraftState> {
        if !validate_private_directory(&self.root)? {
            return Ok(DraftState::default());
        }
        let file = self
            .root
            .join(format!("draft-{}.json", safe_state_component(pane_id)));
        match std::fs::symlink_metadata(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DraftState::default());
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let bytes = read_regular_file(&file)?;
        let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => value,
            Err(error) => return quarantine_draft(&file, error),
        };
        let versioned = value
            .as_object()
            .is_some_and(|object| object.contains_key("version"));
        let draft = if versioned {
            serde_json::from_value(value).map(ReadDraft::Current)
        } else {
            serde_json::from_value(value).map(ReadDraft::Legacy)
        };
        let draft = match draft {
            Ok(draft) => draft,
            Err(error) => return quarantine_draft(&file, error),
        };
        let (session_id, editor, legacy_attachments, prompt_displays) = match draft {
            ReadDraft::Current(draft) if matches!(draft.version, 2 | 3 | DRAFT_VERSION) => {
                let session_id = (draft.version >= 3).then_some(draft.session_id).flatten();
                (
                    session_id,
                    draft.editor,
                    draft.attachments,
                    draft.prompt_displays,
                )
            }
            ReadDraft::Current(draft) => {
                return quarantine_draft(
                    &file,
                    format!("unsupported draft version {}", draft.version),
                );
            }
            ReadDraft::Legacy(draft) => (
                None,
                EditorSnapshot::plain(draft.text),
                draft.attachments,
                Vec::new(),
            ),
        };
        // Drafts written before attachments held a place in the line kept them
        // in a list beside it, drawn above the input. That is where they belong
        // when they move into the line.
        let editor = if legacy_attachments.is_empty() {
            editor
        } else {
            let mut chunks = legacy_attachments
                .into_iter()
                .map(|attachment| EditorChunk::Attachment {
                    id: attachment.id,
                    display: attachment.display,
                })
                .collect::<Vec<_>>();
            chunks.extend(editor.chunks);
            EditorSnapshot { chunks }
        };
        Ok(DraftState {
            session_id,
            text: editor.submission_text(),
            attachments: editor.attachments(),
            editor,
            prompt_displays,
        })
    }

    fn registry_path(&self) -> PathBuf {
        self.root.join("registry.json")
    }

    fn bind_draft_after_verification(&self, source_pane: &str, session_id: &str) -> AppResult<()> {
        let draft_path = self.draft_path(source_pane)?;
        if !draft_path.exists() {
            return Ok(());
        }
        reject_symlink(&draft_path)?;
        let draft = self.load_draft(source_pane)?;
        match draft.session_id.as_deref() {
            Some(saved) if saved == session_id => Ok(()),
            Some(_) => remove_file_if_present(&draft_path),
            None => self.save_editor_draft(
                source_pane,
                Some(session_id),
                &draft.editor,
                &draft.prompt_displays,
            ),
        }
    }

    fn remove_replaced_session_state(
        &self,
        namespace: &PaneNamespaceState,
        live_session: &str,
    ) -> AppResult<()> {
        let safe = checked_pane_component(&namespace.source_pane)?;
        let old_journal = self.history_journal(&namespace.source_pane, &namespace.session_id)?;
        let history_root = self.root.join("history");
        let history_pane = history_root.join(safe);
        require_exact_parent(old_journal.path(), &history_pane)?;
        reject_symlink(&history_root)?;
        reject_symlink(&history_pane)?;
        reject_symlink(old_journal.path())?;

        let draft_path = self.draft_path(&namespace.source_pane)?;
        let keep_draft = if draft_path.exists() {
            reject_symlink(&draft_path)?;
            self.load_draft(&namespace.source_pane)?
                .session_id
                .as_deref()
                == Some(live_session)
        } else {
            false
        };

        remove_file_if_present(old_journal.path())?;
        remove_empty_directory_if_present(&history_pane)?;
        if !keep_draft {
            remove_file_if_present(&draft_path)?;
        }
        self.remove_source(&namespace.source_pane)?;
        remove_file_if_present(&self.namespace_path(&namespace.source_pane)?)?;
        Ok(())
    }

    fn retain_or_expire_orphan(
        &self,
        mut namespace: PaneNamespaceState,
        now_ms: u64,
    ) -> AppResult<()> {
        let orphaned_since = *namespace.orphaned_since_ms.get_or_insert(now_ms);
        if now_ms.saturating_sub(orphaned_since) >= ORPHAN_RETENTION_MS {
            return self.remove_pane_state(&namespace.source_pane);
        }
        self.save_namespace(&namespace)
    }

    fn save_namespace(&self, namespace: &PaneNamespaceState) -> AppResult<()> {
        let panes = self.root.join("panes");
        ensure_private_directory(&self.root)?;
        ensure_private_directory(&panes)?;
        let destination = self.namespace_path(&namespace.source_pane)?;
        reject_symlink(&destination)?;
        atomic_write(&self.root, &destination, serde_json::to_vec(namespace)?)
    }

    fn namespace_path(&self, source_pane: &str) -> AppResult<PathBuf> {
        let panes = self.root.join("panes");
        let path = panes.join(format!("{}.json", checked_pane_component(source_pane)?));
        require_exact_parent(&path, &panes)?;
        Ok(path)
    }

    fn draft_path(&self, source_pane: &str) -> AppResult<PathBuf> {
        let path = self.root.join(format!(
            "draft-{}.json",
            checked_pane_component(source_pane)?
        ));
        require_exact_parent(&path, &self.root)?;
        Ok(path)
    }

    fn load_registry(&self) -> AppResult<OverlayRegistry> {
        let path = self.registry_path();
        reject_symlink(&self.root)?;
        if std::fs::symlink_metadata(&self.root).is_ok_and(|metadata| !metadata.is_dir()) {
            return Ok(OverlayRegistry::default());
        }
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(OverlayRegistry::default());
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        match serde_json::from_slice(&read_regular_file(&path)?) {
            Ok(registry) => Ok(registry),
            Err(error) => {
                let invalid = path.with_extension("json.invalid");
                std::fs::rename(&path, &invalid)?;
                Err(AppError::new(
                    "plugin state",
                    format!("invalid registry moved to {}: {error}", invalid.display()),
                ))
            }
        }
    }

    fn save_registry(&self, registry: &OverlayRegistry) -> AppResult<()> {
        atomic_write(
            &self.root,
            &self.registry_path(),
            serde_json::to_vec(registry)?,
        )
    }
}

fn verified_session(result: &serde_json::Value, expected_pane: &str) -> AppResult<String> {
    let agent = result
        .get("agent")
        .ok_or_else(|| AppError::new("plugin state", "Herdr response has no agent record"))?;
    if agent.get("pane_id").and_then(serde_json::Value::as_str) != Some(expected_pane) {
        return Err(AppError::new(
            "plugin state",
            "Herdr returned a different source pane",
        ));
    }
    let session = agent
        .get("agent_session")
        .ok_or_else(|| AppError::new("plugin state", "native agent session is unavailable"))?;
    if session.get("kind").and_then(serde_json::Value::as_str) != Some("id") {
        return Err(AppError::new(
            "plugin state",
            "native agent session is not id-based",
        ));
    }
    session
        .get("value")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::new("plugin state", "native agent session id is unavailable"))
}

fn checked_pane_component(source_pane: &str) -> AppResult<String> {
    let mut parts = source_pane.split(':');
    let workspace = parts.next().unwrap_or_default();
    let pane = parts.next().unwrap_or_default();
    let valid_part = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
    };
    if !valid_part(workspace) || !valid_part(pane) || parts.next().is_some() {
        return Err(AppError::new(
            "plugin state",
            "invalid source pane identifier",
        ));
    }
    Ok(safe_state_component(source_pane))
}

fn require_exact_parent(path: &Path, expected_parent: &Path) -> AppResult<()> {
    if path.parent() != Some(expected_parent) {
        return Err(AppError::new(
            "plugin state",
            format!("state target escapes its namespace: {}", path.display()),
        ));
    }
    Ok(())
}

pub(crate) fn reject_symlink(path: &Path) -> AppResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::new(
            "plugin state",
            format!("refusing symlinked state target: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn ensure_private_directory(path: &Path) -> AppResult<()> {
    reject_symlink(path)?;
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let directory = open_private_directory(path)?.ok_or_else(|| {
        AppError::new(
            "plugin state",
            format!("state directory disappeared: {}", path.display()),
        )
    })?;
    directory.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub(crate) fn validate_private_directory(path: &Path) -> AppResult<bool> {
    Ok(open_private_directory(path)?.is_some())
}

fn open_private_directory(path: &Path) -> AppResult<Option<File>> {
    reject_symlink(path)?;
    let directory = match OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_DIRECTORY)
        .open(path)
    {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !directory.metadata()?.is_dir() {
        return Err(AppError::new(
            "plugin state",
            format!("state directory is not a directory: {}", path.display()),
        ));
    }
    Ok(Some(directory))
}

pub(crate) fn nofollow_flag() -> i32 {
    O_NOFOLLOW
}

pub(crate) fn validate_regular_file(file: &File, path: &Path) -> AppResult<()> {
    if !file.metadata()?.is_file() {
        return Err(AppError::new(
            "plugin state",
            format!("state target is not a regular file: {}", path.display()),
        ));
    }
    Ok(())
}

fn read_regular_file(path: &Path) -> AppResult<Vec<u8>> {
    reject_symlink(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;
    validate_regular_file(&file, path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn remove_file_if_present(path: &Path) -> AppResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove_directory_if_present(path: &Path) -> AppResult<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove_empty_directory_if_present(path: &Path) -> AppResult<()> {
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn quarantine_draft(file: &Path, error: impl std::fmt::Display) -> AppResult<DraftState> {
    let invalid = file.with_extension("json.invalid");
    std::fs::rename(file, &invalid)?;
    Err(AppError::new(
        "plugin state",
        format!("invalid draft moved to {}: {error}", invalid.display()),
    ))
}

fn atomic_write(root: &Path, destination: &Path, bytes: Vec<u8>) -> AppResult<()> {
    ensure_private_directory(root)?;
    let parent = destination.parent().ok_or_else(|| {
        AppError::new(
            "plugin state",
            format!("state target has no parent: {}", destination.display()),
        )
    })?;
    if parent != root {
        ensure_private_directory(parent)?;
    }
    reject_symlink(destination)?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let (temporary, mut file) = (0..16)
        .find_map(|_| {
            let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temporary = destination
                .with_extension(format!("tmp-{}-{nonce}-{sequence}", std::process::id()));
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .custom_flags(O_NOFOLLOW)
                .open(&temporary)
            {
                Ok(file) => Some(Ok((temporary, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .transpose()?
        .ok_or_else(|| AppError::new("plugin state", "cannot allocate a temporary state file"))?;
    let result = (|| -> AppResult<()> {
        validate_regular_file(&file, &temporary)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        reject_symlink(destination)?;
        std::fs::rename(&temporary, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn safe_state_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("JSON", error.to_string())
    }
}
