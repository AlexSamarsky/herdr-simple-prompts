use crate::agent::{AgentKind, agent_identity, agent_identity_from_response};
use crate::ansi::sanitize_ansi;
use crate::composer::{NativeComposerState, classify_native_composer};
use crate::editor::EditorSnapshot;
use crate::herdr::HerdrClient;
use crate::state::StateStore;
use crate::{AppError, AppResult};
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HAND_BACK_ATTEMPTS: usize = 8;
const HAND_BACK_RETRY_DELAY: Duration = Duration::from_millis(60);

pub fn toggle(client: &HerdrClient, state: &StateStore, current_pane: &str) -> AppResult<()> {
    state.with_lifecycle_lock(|| toggle_unlocked(client, state, current_pane))
}

fn toggle_unlocked(client: &HerdrClient, state: &StateStore, current_pane: &str) -> AppResult<()> {
    if let Some(source) = state.source_for_overlay(current_pane)? {
        match client.pane_get(current_pane) {
            Ok(_) => {}
            Err(error) if error.is_pane_not_found() => {
                return recover_stale_overlay_context(client, state, &source);
            }
            Err(error) => return Err(AppError::new("toggle", error.to_string())),
        }
        // Best effort, and deliberately not fatal: a draft that cannot be
        // handed back stays in the overlay, where the next open will offer it
        // again. Failing the toggle would strand the user in the overlay.
        let _ = hand_back_overlay_draft(client, state, &source);
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

fn hand_back_overlay_draft(
    client: &HerdrClient,
    state: &StateStore,
    source: &str,
) -> AppResult<()> {
    let draft = state.load_draft(source)?;
    // Attachments live in the native composer already; a draft carrying them
    // has nothing to move and must not be cleared.
    if draft.text.trim().is_empty() || !draft.attachments.is_empty() {
        return Ok(());
    }
    let identity = agent_identity(client, source)?;
    let handed_back = hand_back_draft(
        identity.kind,
        &draft.text,
        || {
            client
                .pane_read_visible_ansi(source, 200)
                .map_err(|error| AppError::new("hand back draft", error.to_string()))
        },
        |text| {
            client
                .pane_send_input(source, Some(text), &[])
                .map(|_| ())
                .map_err(|error| AppError::new("hand back draft", error.to_string()))
        },
        HAND_BACK_ATTEMPTS,
        HAND_BACK_RETRY_DELAY,
    )?;
    if handed_back {
        state.save_editor_draft(
            source,
            draft.session_id.as_deref(),
            &EditorSnapshot::default(),
            &[],
            &[],
        )?;
    }
    Ok(())
}

/// Moves the overlay's draft into the native composer as the overlay closes.
///
/// The multiplexer delivers text as a paste, so a multi-line draft lands in the
/// composer whole instead of submitting itself line by line.
///
/// Only an empty native composer is written to: appending to something already
/// being typed there would splice two prompts into one. The overlay's copy is
/// dropped only once the text is seen to have arrived, so a draft is never lost
/// between the two composers.
fn hand_back_draft(
    kind: AgentKind,
    text: &str,
    mut read: impl FnMut() -> AppResult<String>,
    mut send: impl FnMut(&str) -> AppResult<()>,
    attempts: usize,
    retry_delay: Duration,
) -> AppResult<bool> {
    if text.trim().is_empty() {
        return Ok(false);
    }
    if !native_composer_is_clear(kind, &read()?) {
        return Ok(false);
    }
    send(text)?;
    for _ in 0..attempts {
        thread::sleep(retry_delay);
        if !native_composer_is_clear(kind, &read()?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn native_composer_is_clear(kind: AgentKind, ansi: &str) -> bool {
    classify_native_composer(kind, &sanitize_ansi(ansi)) == NativeComposerState::Clear
}

fn recover_stale_overlay_context(
    client: &HerdrClient,
    state: &StateStore,
    source: &str,
) -> AppResult<()> {
    match client.pane_get(source) {
        Ok(_) => {}
        Err(error) if error.is_pane_not_found() => {
            state.remove_source(source)?;
            return Err(AppError::new(
                "toggle",
                format!("source pane {source} no longer exists"),
            ));
        }
        Err(error) => return Err(AppError::new("toggle", error.to_string())),
    }
    let identity_response = match client.agent_get(source) {
        Ok(response) => response,
        Err(error) if error.is_pane_not_found() => {
            return remove_missing_source_mapping(state, source);
        }
        Err(error) => return Err(AppError::new("agent", error.to_string())),
    };
    let identity = agent_identity_from_response(&identity_response, source)?;
    state.remove_source(source)?;
    open_verified_overlay(client, state, source, &identity.session_id)
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
    open_verified_overlay(client, state, source, &identity.session_id)
}

fn open_verified_overlay(
    client: &HerdrClient,
    state: &StateStore,
    source: &str,
    session_id: &str,
) -> AppResult<()> {
    let overlay = client
        .plugin_pane_open_targeted(source)
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
    state.with_lifecycle_lock(|| {
        state.validate_saved_namespaces(&client, now_ms())?;
        toggle_unlocked(&client, &state, &current_pane)
    })
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

#[cfg(test)]
mod tests {
    use super::hand_back_draft;
    use crate::agent::AgentKind;
    use std::cell::Cell;
    use std::time::Duration;

    const OCCUPIED: &str = concat!(
        "• answer\n",
        "────────\n",
        "› already typing\n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    );
    const EMPTY: &str = concat!(
        "• answer\n",
        "────────\n",
        "› \n",
        "gpt-5.6-sol xhigh · /repo · weekly 75% left",
    );

    #[test]
    fn a_draft_moves_into_an_empty_native_composer() {
        let sent = Cell::new(0);
        let handed_back = hand_back_draft(
            AgentKind::Codex,
            "half written prompt",
            || {
                Ok(if sent.get() == 0 {
                    EMPTY.to_owned()
                } else {
                    OCCUPIED.to_owned()
                })
            },
            |_| {
                sent.set(sent.get() + 1);
                Ok(())
            },
            8,
            Duration::ZERO,
        )
        .unwrap();

        assert!(handed_back);
        assert_eq!(sent.get(), 1);
    }

    /// Appending to a composer someone is already typing in would splice two
    /// prompts into one.
    #[test]
    fn an_occupied_native_composer_is_left_alone() {
        let sent = Cell::new(0);
        let handed_back = hand_back_draft(
            AgentKind::Codex,
            "half written prompt",
            || Ok(OCCUPIED.to_owned()),
            |_| {
                sent.set(sent.get() + 1);
                Ok(())
            },
            8,
            Duration::ZERO,
        )
        .unwrap();

        assert!(!handed_back);
        assert_eq!(sent.get(), 0, "nothing may be written over live input");
    }

    #[test]
    fn an_empty_draft_touches_nothing() {
        let reads = Cell::new(0);
        let handed_back = hand_back_draft(
            AgentKind::Codex,
            "   \n  ",
            || {
                reads.set(reads.get() + 1);
                Ok(EMPTY.to_owned())
            },
            |_| panic!("an empty draft must not be sent"),
            8,
            Duration::ZERO,
        )
        .unwrap();

        assert!(!handed_back);
        assert_eq!(reads.get(), 0);
    }

    /// The overlay keeps its copy unless the text is seen to have arrived.
    #[test]
    fn a_draft_that_does_not_arrive_is_not_reported_as_handed_back() {
        let handed_back = hand_back_draft(
            AgentKind::Codex,
            "half written prompt",
            || Ok(EMPTY.to_owned()),
            |_| Ok(()),
            3,
            Duration::ZERO,
        )
        .unwrap();

        assert!(!handed_back);
    }
}
