use crate::agent::AgentIdentity;
use crate::agent::follower::{FollowerEvent, TranscriptFollower};
use crate::herdr::HerdrClient;
use crate::model::Attachment;
use crate::transport::AgentTransport;
use crate::{AppError, AppResult};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const EVENT_QUEUE_CAPACITY: usize = 64;
const ACTION_QUEUE_CAPACITY: usize = 16;
const FOLLOWER_QUEUE_CAPACITY: usize = 4;

#[derive(Debug)]
enum ActionCommand {
    Submit {
        local_id: String,
        text: String,
    },
    Interrupt,
    LocalImage {
        attachment: Attachment,
    },
    StagedImage {
        attachment: Attachment,
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug)]
enum FollowerCommand {
    FinalizePending,
}

#[derive(Debug)]
pub enum RuntimeEvent {
    Transcript(Vec<FollowerEvent>),
    TranscriptError(String),
    Observation(Result<(AgentIdentity, String), String>),
    Submitted {
        local_id: String,
        result: Result<(), String>,
    },
    Interrupted(Result<(), String>),
    ImageForwarded {
        attachment: Attachment,
        result: Result<(), String>,
    },
}

pub struct UiRuntime {
    action_tx: SyncSender<ActionCommand>,
    follower_tx: SyncSender<FollowerCommand>,
    events: Receiver<RuntimeEvent>,
    stop: Arc<AtomicBool>,
    _threads: Vec<JoinHandle<()>>,
}

impl UiRuntime {
    pub fn spawn(
        socket: &Path,
        identity: AgentIdentity,
        follower: TranscriptFollower,
    ) -> AppResult<Self> {
        let (event_tx, events) = sync_channel(EVENT_QUEUE_CAPACITY);
        let (action_tx, action_rx) = sync_channel(ACTION_QUEUE_CAPACITY);
        let (follower_tx, follower_rx) = sync_channel(FOLLOWER_QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));

        let observer_transport = AgentTransport::new(
            HerdrClient::connect(socket).map_err(|error| AppError::new("ui", error.to_string()))?,
            identity.clone(),
        );
        let action_transport = AgentTransport::new(
            HerdrClient::connect(socket).map_err(|error| AppError::new("ui", error.to_string()))?,
            identity,
        );

        let threads = vec![
            spawn_observer(Arc::clone(&stop), event_tx.clone(), observer_transport),
            spawn_follower(Arc::clone(&stop), event_tx.clone(), follower_rx, follower),
            spawn_actions(Arc::clone(&stop), event_tx, action_rx, action_transport),
        ];

        Ok(Self {
            action_tx,
            follower_tx,
            events,
            stop,
            _threads: threads,
        })
    }

    pub fn try_recv(&self) -> Option<RuntimeEvent> {
        self.events.try_recv().ok()
    }

    pub fn submit(&self, local_id: String, text: String) -> AppResult<()> {
        self.send_action(ActionCommand::Submit { local_id, text })
    }

    pub fn interrupt(&self) -> AppResult<()> {
        self.send_action(ActionCommand::Interrupt)
    }

    pub fn forward_local_image(&self, attachment: Attachment) -> AppResult<()> {
        self.send_action(ActionCommand::LocalImage { attachment })
    }

    pub fn forward_staged_image(&self, attachment: Attachment, path: PathBuf) -> AppResult<()> {
        self.send_action(ActionCommand::StagedImage { attachment, path })
    }

    pub fn finalize_pending(&self) {
        let _ = self.follower_tx.try_send(FollowerCommand::FinalizePending);
    }

    fn send_action(&self, command: ActionCommand) -> AppResult<()> {
        self.action_tx
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => AppError::new("ui", "agent action queue is full"),
                TrySendError::Disconnected(_) => {
                    AppError::new("ui", "agent action worker has stopped")
                }
            })
    }
}

impl Drop for UiRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn spawn_observer(
    stop: Arc<AtomicBool>,
    events: SyncSender<RuntimeEvent>,
    transport: AgentTransport,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let observation = transport
                .refresh_identity()
                .and_then(|identity| transport.visible_source(8).map(|screen| (identity, screen)))
                .map_err(|error| error.to_string());
            let _ = events.try_send(RuntimeEvent::Observation(observation));
            thread::sleep(Duration::from_millis(200));
        }
    })
}

fn spawn_follower(
    stop: Arc<AtomicBool>,
    events: SyncSender<RuntimeEvent>,
    commands: Receiver<FollowerCommand>,
    mut follower: TranscriptFollower,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            while let Ok(FollowerCommand::FinalizePending) = commands.try_recv() {
                if let Some(event) = follower.finalize_pending() {
                    let _ = events.send(RuntimeEvent::Transcript(vec![event]));
                }
            }
            match follower.poll() {
                Ok(items) if !items.is_empty() => {
                    let _ = events.send(RuntimeEvent::Transcript(items));
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = events.send(RuntimeEvent::TranscriptError(error.to_string()));
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    })
}

fn spawn_actions(
    stop: Arc<AtomicBool>,
    events: SyncSender<RuntimeEvent>,
    commands: Receiver<ActionCommand>,
    transport: AgentTransport,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let command = match commands.recv_timeout(Duration::from_millis(100)) {
                Ok(command) => command,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let event = match command {
                ActionCommand::Submit { local_id, text } => RuntimeEvent::Submitted {
                    local_id,
                    result: transport.submit(&text).map_err(|error| error.to_string()),
                },
                ActionCommand::Interrupt => RuntimeEvent::Interrupted(
                    transport.interrupt().map_err(|error| error.to_string()),
                ),
                ActionCommand::LocalImage { attachment } => RuntimeEvent::ImageForwarded {
                    attachment,
                    result: transport
                        .forward_local_image_paste()
                        .map_err(|error| error.to_string()),
                },
                ActionCommand::StagedImage { attachment, path } => RuntimeEvent::ImageForwarded {
                    attachment,
                    result: transport
                        .forward_staged_image(&path)
                        .map_err(|error| error.to_string()),
                },
            };
            let _ = events.send(event);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{ActionCommand, UiRuntime};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::sync_channel;
    use std::time::{Duration, Instant};

    #[test]
    fn submit_only_enqueues_and_never_waits_for_herdr_io() {
        let (action_tx, action_rx) = sync_channel(1);
        let (follower_tx, _follower_rx) = sync_channel(1);
        let (_event_tx, events) = sync_channel(1);
        let runtime = UiRuntime {
            action_tx,
            follower_tx,
            events,
            stop: Arc::new(AtomicBool::new(false)),
            _threads: Vec::new(),
        };

        let started = Instant::now();
        runtime.submit("local-1".into(), "hello".into()).unwrap();

        assert!(started.elapsed() < Duration::from_millis(20));
        assert!(matches!(
            action_rx.try_recv().unwrap(),
            ActionCommand::Submit { local_id, text }
                if local_id == "local-1" && text == "hello"
        ));
    }

    #[test]
    fn full_action_queue_fails_instead_of_blocking_the_ui() {
        let (action_tx, _action_rx) = sync_channel(1);
        let (follower_tx, _follower_rx) = sync_channel(1);
        let (_event_tx, events) = sync_channel(1);
        let runtime = UiRuntime {
            action_tx,
            follower_tx,
            events,
            stop: Arc::new(AtomicBool::new(false)),
            _threads: Vec::new(),
        };
        runtime.submit("local-1".into(), "first".into()).unwrap();

        let started = Instant::now();
        let error = runtime
            .submit("local-2".into(), "second".into())
            .unwrap_err();

        assert!(started.elapsed() < Duration::from_millis(20));
        assert!(error.to_string().contains("queue is full"));
    }
}
