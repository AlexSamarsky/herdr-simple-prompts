use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize, Serialize)]
struct OverlayRegistry {
    overlays: BTreeMap<String, String>,
}

pub struct StateStore {
    root: PathBuf,
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

    pub fn save_draft(&self, pane_id: &str, text: &str) -> AppResult<()> {
        let file = self
            .root
            .join(format!("draft-{}.json", safe_pane_id(pane_id)));
        atomic_write(&self.root, &file, serde_json::to_vec(&text)?)
    }

    pub fn load_draft(&self, pane_id: &str) -> AppResult<String> {
        let file = self
            .root
            .join(format!("draft-{}.json", safe_pane_id(pane_id)));
        if !file.exists() {
            return Ok(String::new());
        }
        serde_json::from_slice(&std::fs::read(&file)?).map_err(|error| {
            AppError::new(
                "plugin state",
                format!("invalid {}: {error}", file.display()),
            )
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
