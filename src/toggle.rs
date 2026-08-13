use crate::agent::{agent_identity, agent_identity_from_response};
use crate::herdr::HerdrClient;
use crate::state::StateStore;
use crate::{AppError, AppResult};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn toggle(client: &HerdrClient, state: &StateStore, current_pane: &str) -> AppResult<()> {
    if let Some(source) = state.source_for_overlay(current_pane)? {
        match client.pane_get(current_pane) {
            Ok(_) => {}
            Err(error) if error.is_pane_not_found() => {
                return recover_stale_overlay_context(client, state, &source);
            }
            Err(error) => return Err(AppError::new("toggle", error.to_string())),
        }
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
            Err(error) if error.is_pane_not_found() => {
                state.remove_source(current_pane)?;
            }
            Err(error) => return Err(AppError::new("toggle", error.to_string())),
        }
    }

    open_overlay(client, state, current_pane)
}

fn recover_stale_overlay_context(
    client: &HerdrClient,
    state: &StateStore,
    source: &str,
) -> AppResult<()> {
    let workspace_id = match client.pane_workspace_id(source) {
        Ok(workspace_id) => workspace_id,
        Err(error) if error.is_pane_not_found() => {
            state.remove_source(source)?;
            return Err(AppError::new(
                "toggle",
                format!("source pane {source} no longer exists"),
            ));
        }
        Err(error) => return Err(AppError::new("toggle", error.to_string())),
    };
    let identity_response = match client.agent_get(source) {
        Ok(response) => response,
        Err(error) if error.is_pane_not_found() => {
            return remove_missing_source_mapping(state, source);
        }
        Err(error) => return Err(AppError::new("agent", error.to_string())),
    };
    let identity = agent_identity_from_response(&identity_response, source)?;
    match client.pane_focus(source) {
        Ok(_) => {}
        Err(error) if error.is_pane_not_found() => {
            return remove_missing_source_mapping(state, source);
        }
        Err(error) => return Err(AppError::new("toggle", error.to_string())),
    }
    state.remove_source(source)?;
    open_verified_overlay(client, state, source, &identity.session_id, &workspace_id)
}

fn remove_missing_source_mapping(state: &StateStore, source: &str) -> AppResult<()> {
    state.remove_source(source)?;
    Err(AppError::new(
        "toggle",
        format!("source pane {source} no longer exists"),
    ))
}

fn open_overlay(client: &HerdrClient, state: &StateStore, source: &str) -> AppResult<()> {
    let identity = agent_identity(client, source)?;
    let workspace_id = client
        .pane_workspace_id(source)
        .map_err(|error| AppError::new("toggle", error.to_string()))?;
    open_verified_overlay(client, state, source, &identity.session_id, &workspace_id)
}

fn open_verified_overlay(
    client: &HerdrClient,
    state: &StateStore,
    source: &str,
    session_id: &str,
    workspace_id: &str,
) -> AppResult<()> {
    let overlay = client
        .plugin_pane_open_anchored(source, workspace_id)
        .map_err(|error| AppError::new("toggle", error.to_string()))?;
    if let Err(save_error) = state.save_overlay(source, &overlay) {
        if let Err(close_error) = client.plugin_pane_close(&overlay) {
            return Err(AppError::new(
                "toggle",
                format!(
                    "cannot persist overlay registry ({save_error}); cleanup also failed: {close_error}"
                ),
            ));
        }
        return Err(save_error);
    }
    if let Err(bind_error) = state.bind_verified_namespace(source, session_id, now_ms()) {
        let _ = state.remove_source(source);
        let _ = client.plugin_pane_close(&overlay);
        return Err(bind_error);
    }
    Ok(())
}

pub fn run_from_env() -> AppResult<()> {
    let socket = required_env("HERDR_SOCKET_PATH")?;
    let state_root = required_env("HERDR_PLUGIN_STATE_DIR")?;
    let current_pane = required_env("HERDR_PANE_ID")?;
    let client = HerdrClient::connect(Path::new(&socket))
        .map_err(|error| AppError::new("toggle", error.to_string()))?;
    let state = StateStore::at(state_root);
    state.validate_saved_namespaces(&client, now_ms())?;
    toggle(&client, &state, &current_pane)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn required_env(name: &'static str) -> AppResult<String> {
    std::env::var(name).map_err(|_| AppError::new("toggle", format!("{name} is not set")))
}
