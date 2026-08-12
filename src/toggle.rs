use crate::agent::agent_identity;
use crate::herdr::HerdrClient;
use crate::state::StateStore;
use crate::{AppError, AppResult};
use std::path::Path;

pub fn toggle(client: &HerdrClient, state: &StateStore, current_pane: &str) -> AppResult<()> {
    if let Some(source) = state.source_for_overlay(current_pane)? {
        client
            .pane_get(current_pane)
            .map_err(|error| AppError::new("toggle", error.to_string()))?;
        client
            .plugin_pane_close(current_pane)
            .map_err(|error| AppError::new("toggle", error.to_string()))?;
        client
            .pane_focus(&source)
            .map_err(|error| AppError::new("toggle", error.to_string()))?;
        state.remove_source(&source)?;
        return Ok(());
    }

    if let Some(overlay) = state.overlay_for_source(current_pane)? {
        match client.pane_get(&overlay) {
            Ok(_) => {
                client
                    .plugin_pane_focus(&overlay)
                    .map_err(|error| AppError::new("toggle", error.to_string()))?;
                return Ok(());
            }
            Err(error) if error.api_code() == Some("not_found") => {
                state.remove_source(current_pane)?;
            }
            Err(error) => return Err(AppError::new("toggle", error.to_string())),
        }
    }

    agent_identity(client, current_pane)?;
    let overlay = client
        .plugin_pane_open(current_pane)
        .map_err(|error| AppError::new("toggle", error.to_string()))?;
    state.save_overlay(current_pane, &overlay)
}

pub fn run_from_env() -> AppResult<()> {
    let socket = required_env("HERDR_SOCKET_PATH")?;
    let state_root = required_env("HERDR_PLUGIN_STATE_DIR")?;
    let current_pane = required_env("HERDR_PANE_ID")?;
    let client = HerdrClient::connect(Path::new(&socket))
        .map_err(|error| AppError::new("toggle", error.to_string()))?;
    toggle(&client, &StateStore::at(state_root), &current_pane)
}

fn required_env(name: &'static str) -> AppResult<String> {
    std::env::var(name).map_err(|_| AppError::new("toggle", format!("{name} is not set")))
}
