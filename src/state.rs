use crate::editor::EditorSnapshot;
use crate::model::Attachment;
use crate::paste::CompactPromptOverride;
use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

#[derive(Default, Deserialize, Serialize)]
struct OverlayRegistry {
    overlays: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DraftState {
    pub text: String,
    pub editor: EditorSnapshot,
    pub attachments: Vec<Attachment>,
    pub prompt_displays: Vec<CompactPromptOverride>,
}

#[derive(Deserialize, Serialize)]
struct PersistedDraft {
    version: u8,
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

#[derive(Clone)]
pub struct StateStore {
    root: PathBuf,
}

struct DraftSnapshot {
    editor: EditorSnapshot,
    attachments: Vec<Attachment>,
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
    pub fn spawn(store: StateStore, pane_id: String) -> Self {
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
                    &snapshot.editor,
                    &snapshot.attachments,
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
        attachments: Vec<Attachment>,
        prompt_displays: Vec<CompactPromptOverride>,
    ) {
        let (lock, ready) = &*self.slot;
        let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending = Some(DraftSnapshot {
            editor,
            attachments,
            prompt_displays,
        });
        ready.notify_one();
    }

    pub fn queue(&self, text: String, attachments: Vec<Attachment>) {
        self.queue_editor(EditorSnapshot::plain(text), attachments, Vec::new());
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

    pub fn save_overlay(&self, source: &str, overlay: &str) -> AppResult<()> {
        let mut registry = self.load_registry()?;
        registry
            .overlays
            .insert(source.to_owned(), overlay.to_owned());
        self.save_registry(&registry)
    }

    pub fn remove_source(&self, source: &str) -> AppResult<()> {
        let mut registry = self.load_registry()?;
        registry.overlays.remove(source);
        self.save_registry(&registry)
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
        self.save_editor_draft(pane_id, &EditorSnapshot::plain(text), attachments, &[])
    }

    pub fn save_editor_draft(
        &self,
        pane_id: &str,
        editor: &EditorSnapshot,
        attachments: &[Attachment],
        prompt_displays: &[CompactPromptOverride],
    ) -> AppResult<()> {
        let file = self
            .root
            .join(format!("draft-{}.json", safe_pane_id(pane_id)));
        atomic_write(
            &self.root,
            &file,
            serde_json::to_vec(&PersistedDraft {
                version: 2,
                editor: editor.clone(),
                attachments: attachments
                    .iter()
                    .map(|attachment| PersistedAttachment {
                        id: attachment.id.clone(),
                        display: attachment.display.clone(),
                    })
                    .collect(),
                prompt_displays: prompt_displays.to_vec(),
            })?,
        )
    }

    pub fn load_draft(&self, pane_id: &str) -> AppResult<DraftState> {
        let file = self
            .root
            .join(format!("draft-{}.json", safe_pane_id(pane_id)));
        if !file.exists() {
            return Ok(DraftState::default());
        }
        let bytes = std::fs::read(&file)?;
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
        let (editor, attachments, prompt_displays) = match draft {
            ReadDraft::Current(draft) if draft.version == 2 => {
                (draft.editor, draft.attachments, draft.prompt_displays)
            }
            ReadDraft::Current(draft) => {
                return quarantine_draft(
                    &file,
                    format!("unsupported draft version {}", draft.version),
                );
            }
            ReadDraft::Legacy(draft) => (
                EditorSnapshot::plain(draft.text),
                draft.attachments,
                Vec::new(),
            ),
        };
        let text = editor.submission_text();
        Ok(DraftState {
            text,
            editor,
            attachments: attachments
                .into_iter()
                .map(|attachment| Attachment {
                    id: attachment.id,
                    display: attachment.display,
                    native_path: None,
                })
                .collect(),
            prompt_displays,
        })
    }

    fn registry_path(&self) -> PathBuf {
        self.root.join("registry.json")
    }

    fn load_registry(&self) -> AppResult<OverlayRegistry> {
        let path = self.registry_path();
        if !path.exists() {
            return Ok(OverlayRegistry::default());
        }
        match serde_json::from_slice(&std::fs::read(&path)?) {
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

fn quarantine_draft(file: &Path, error: impl std::fmt::Display) -> AppResult<DraftState> {
    let invalid = file.with_extension("json.invalid");
    std::fs::rename(file, &invalid)?;
    Err(AppError::new(
        "plugin state",
        format!("invalid draft moved to {}: {error}", invalid.display()),
    ))
}

fn atomic_write(root: &Path, destination: &Path, bytes: Vec<u8>) -> AppResult<()> {
    std::fs::create_dir_all(root)?;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    let temporary = destination.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary, destination)?;
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn safe_pane_id(pane_id: &str) -> String {
    pane_id
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
