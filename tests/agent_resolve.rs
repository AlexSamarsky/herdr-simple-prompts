mod support;

use herdr_simple_prompts::agent::{AgentKind, AgentPaths, agent_identity, resolve_transcript};
use herdr_simple_prompts::herdr::HerdrClient;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TREE: AtomicU64 = AtomicU64::new(1);

struct TestTree(PathBuf);

impl TestTree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "herdr-simple-prompts-resolve-{}-{}",
            std::process::id(),
            NEXT_TREE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    fn file(&self, relative: &str) -> PathBuf {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{}\n").unwrap();
        path
    }
}

impl Drop for TestTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn resolves_codex_session_by_native_id_under_codex_home() {
    let root = TestTree::new();
    let wanted = root.file("codex/sessions/2026/08/12/rollout-x-session-123.jsonl");
    let paths = AgentPaths::new(root.path("home"), Some(root.path("codex")), None);

    assert_eq!(
        resolve_transcript(AgentKind::Codex, "session-123", &paths).unwrap(),
        wanted
    );
}

#[test]
fn resolves_claude_exact_filename_and_ignores_similar_session() {
    let root = TestTree::new();
    root.file("claude/projects/a/not-session-123.jsonl");
    let wanted = root.file("claude/projects/a/session-123.jsonl");
    let paths = AgentPaths::new(root.path("home"), None, Some(root.path("claude")));

    assert_eq!(
        resolve_transcript(AgentKind::Claude, "session-123", &paths).unwrap(),
        wanted
    );
}

#[test]
fn rejects_unsafe_or_ambiguous_session_resolution() {
    let root = TestTree::new();
    root.file("codex/sessions/a/rollout-session-123.jsonl");
    root.file("codex/sessions/b/rollout-session-123.jsonl");
    let paths = AgentPaths::new(root.path("home"), Some(root.path("codex")), None);

    assert!(resolve_transcript(AgentKind::Codex, "../escape", &paths).is_err());
    assert!(resolve_transcript(AgentKind::Codex, "session-123", &paths).is_err());
}

#[test]
fn reads_supported_agent_identity_from_herdr() {
    let fake = support::FakeHerdr::start(|request| {
        assert_eq!(request["method"], "agent.get");
        serde_json::json!({
            "id": request["id"],
            "result": {
                "type": "agent_info",
                "agent": {
                    "pane_id": "w1:p1",
                    "agent": "codex",
                    "agent_status": "working",
                    "foreground_cwd": "/tmp/project",
                    "cwd": "/tmp",
                    "agent_session": {
                        "source": "herdr:codex",
                        "agent": "codex",
                        "kind": "id",
                        "value": "session-123"
                    }
                }
            }
        })
    });
    let client = HerdrClient::connect(fake.socket_path()).unwrap();

    let identity = agent_identity(&client, "w1:p1").unwrap();

    assert_eq!(identity.kind, AgentKind::Codex);
    assert_eq!(identity.session_id, "session-123");
    assert_eq!(identity.cwd, Path::new("/tmp/project"));
    assert!(identity.status.is_working());
}
