use crate::agent::follower::{FollowerEvent, TranscriptFollower};
use crate::agent::{AgentIdentity, AgentStatus};
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
        let stop = Arc::new(AtomicBool::new(false));
        let agent_working = Arc::new(AtomicBool::new(identity.status.is_working()));

        let observer_transport = AgentTransport::new(
            HerdrClient::connect(socket).map_err(|error| AppError::new("ui", error.to_string()))?,
            identity.clone(),
        );
        let action_transport = AgentTransport::new(
            HerdrClient::connect(socket).map_err(|error| AppError::new("ui", error.to_string()))?,
            identity,
        );

        let threads = vec![
            spawn_observer(
                Arc::clone(&stop),
                Arc::clone(&agent_working),
                event_tx.clone(),
                observer_transport,
            ),
            spawn_follower(Arc::clone(&stop), agent_working, event_tx.clone(), follower),
            spawn_actions(Arc::clone(&stop), event_tx, action_rx, action_transport),
        ];

        Ok(Self {
            action_tx,
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
    agent_working: Arc<AtomicBool>,
    events: SyncSender<RuntimeEvent>,
    transport: AgentTransport,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let observation = match transport.refresh_identity() {
                Ok(identity) => {
                    agent_working.store(identity.status.is_working(), Ordering::Release);
                    transport
                        .visible_source(8)
                        .map(|screen| (identity, screen))
                        .map_err(|error| error.to_string())
                }
                Err(error) => Err(error.to_string()),
            };
            let _ = events.try_send(RuntimeEvent::Observation(observation));
            thread::sleep(Duration::from_millis(200));
        }
    })
}

fn spawn_follower(
    stop: Arc<AtomicBool>,
    agent_working: Arc<AtomicBool>,
    events: SyncSender<RuntimeEvent>,
    mut follower: TranscriptFollower,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let status = if agent_working.load(Ordering::Acquire) {
                AgentStatus::Working
            } else {
                AgentStatus::Done
            };
            match follower.poll_for_status(status) {
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
    use super::{ActionCommand, RuntimeEvent, UiRuntime, spawn_follower};
    use crate::agent::claude::ClaudeAdapter;
    use crate::agent::follower::TranscriptFollower;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::sync_channel;
    use std::time::{Duration, Instant};

    #[test]
    fn submit_only_enqueues_and_never_waits_for_herdr_io() {
        let (action_tx, action_rx) = sync_channel(1);
        let (_event_tx, events) = sync_channel(1);
        let runtime = UiRuntime {
            action_tx,
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
        let (_event_tx, events) = sync_channel(1);
        let runtime = UiRuntime {
            action_tx,
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

    #[test]
    fn follower_finalizes_from_atomic_status_even_if_ui_observation_is_dropped() {
        let path = std::env::temp_dir().join(format!(
            "herdr-simple-prompts-runtime-claude-{}.jsonl",
            std::process::id()
        ));
        std::fs::write(
            &path,
            std::fs::read("tests/fixtures/claude/simple.jsonl").unwrap(),
        )
        .unwrap();
        let follower = TranscriptFollower::new(&path, Box::new(ClaudeAdapter::default())).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let working = Arc::new(AtomicBool::new(true));
        let (events_tx, events_rx) = sync_channel(8);
        let worker = spawn_follower(Arc::clone(&stop), Arc::clone(&working), events_tx, follower);

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            RuntimeEvent::Transcript(events) if events.len() == 1
        ));
        working.store(false, std::sync::atomic::Ordering::Release);
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            RuntimeEvent::Transcript(events) if events.len() == 1
        ));

        stop.store(true, std::sync::atomic::Ordering::Release);
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
