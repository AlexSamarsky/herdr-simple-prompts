# Overlay Registry Transaction Lock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent concurrent Simple Prompts processes from losing a live overlay mapping and leaving an orphan pane visible after the next toggle.

**Architecture:** `StateStore` will own a bounded advisory lifecycle lock backed by the Unix `flock` API already used by the history journal. Toggle decisions, startup validation/binding, and source cleanup will each run as one serialized transaction while the existing atomic JSON writes continue to provide crash-safe persistence.

**Tech Stack:** Rust 2024, Rust 1.85 MSRV, Unix domain API integration, `flock`, existing `StateStore`, existing scripted Herdr test server.

---

## File map

- Modify `src/state.rs`: implement the scoped lifecycle lock without changing the registry schema.
- Modify `src/toggle.rs`: serialize the full toggle decision and the validation-plus-toggle entrypoint.
- Modify `src/ui/mod.rs`: serialize startup validation/binding and source cleanup.
- Modify `tests/toggle_state.rs`: prove lifecycle mutations serialize and a toggle cannot make a pane decision while another lifecycle transaction is active.

### Task 1: Add the process-safe lifecycle lock

**Files:**
- Modify: `tests/toggle_state.rs`
- Modify: `src/state.rs`

**Required skill checkpoints:**
- Use `superpowers:test-driven-development` before production changes.
- Use `superpowers:requesting-code-review` after the lock implementation batch.
- Use `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Write the failing serialization test**

Add a test that holds the lifecycle lock in one thread, starts a second mutation,
and requires the second thread to remain outside the critical section until the
first thread releases it:

```rust
#[test]
fn lifecycle_lock_serializes_registry_mutations() {
    let directory = test_state_directory("lifecycle-lock");
    let _ = std::fs::remove_dir_all(&directory);
    let first_store = StateStore::at(&directory);
    let second_store = first_store.clone();
    let observer = first_store.clone();
    let (first_entered_tx, first_entered_rx) = std::sync::mpsc::channel();
    let (release_first_tx, release_first_rx) = std::sync::mpsc::channel();
    let (second_entered_tx, second_entered_rx) = std::sync::mpsc::channel();

    let first = std::thread::spawn(move || {
        first_store
            .with_lifecycle_lock(|| {
                first_store.save_overlay("w1:p1", "w1:p9")?;
                first_entered_tx.send(()).unwrap();
                release_first_rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
    });
    first_entered_rx.recv().unwrap();

    let second = std::thread::spawn(move || {
        second_store
            .with_lifecycle_lock(|| {
                second_entered_tx.send(()).unwrap();
                second_store.save_overlay("w2:p1", "w2:p9")
            })
            .unwrap();
    });

    assert!(
        second_entered_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    release_first_tx.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();

    assert_eq!(
        observer.overlay_for_source("w1:p1").unwrap().as_deref(),
        Some("w1:p9")
    );
    assert_eq!(
        observer.overlay_for_source("w2:p1").unwrap().as_deref(),
        Some("w2:p9")
    );
    std::fs::remove_dir_all(directory).unwrap();
}
```

- [ ] **Step 2: Run the focused test and observe RED**

Run:

```bash
cargo test --locked --test toggle_state lifecycle_lock_serializes_registry_mutations -- --exact
```

Expected: compilation fails because `StateStore::with_lifecycle_lock` does not
exist. This is the intended RED result.

- [ ] **Step 3: Implement the minimal lifecycle lock in `src/state.rs`**

Add these imports and bounded timing constants:

```rust
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LIFECYCLE_LOCK_WAIT_LIMIT: Duration = Duration::from_secs(2);
const LIFECYCLE_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);
```

Add this public scoped operation on `StateStore`:

```rust
pub fn with_lifecycle_lock<T>(
    &self,
    operation: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    let _lock = LifecycleLock::acquire(&self.root)?;
    operation()
}
```

Implement `LifecycleLock` with a private `.lifecycle.lock` file using the
following code:

```rust
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

const STATE_LOCK_EX: std::os::raw::c_int = 2;
const STATE_LOCK_UN: std::os::raw::c_int = 8;
const STATE_LOCK_NB: std::os::raw::c_int = 4;

fn state_flock(
    fd: std::os::raw::c_int,
    operation: std::os::raw::c_int,
) -> std::io::Result<()> {
    unsafe extern "C" {
        #[link_name = "flock"]
        fn os_flock(
            fd: std::os::raw::c_int,
            operation: std::os::raw::c_int,
        ) -> std::os::raw::c_int;
    }

    if unsafe { os_flock(fd, operation) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
```

Do not modify `registry.json` serialization.

- [ ] **Step 4: Run the focused test and observe GREEN**

Run:

```bash
cargo test --locked --test toggle_state lifecycle_lock_serializes_registry_mutations -- --exact
```

Expected: one test passes and both overlay mappings remain present.

- [ ] **Step 5: Run the state/toggle test target**

Run:

```bash
cargo test --locked --test toggle_state
```

Expected: every `toggle_state` test passes.

- [ ] **Step 6: Review and commit the lock batch**

Invoke `superpowers:requesting-code-review`, address only findings within this
task, rerun the focused test, then commit:

```bash
git add src/state.rs tests/toggle_state.rs
git commit -m "add lifecycle state lock"
```

### Task 2: Put every overlay lifecycle path inside the lock

**Files:**
- Modify: `tests/toggle_state.rs`
- Modify: `src/toggle.rs`
- Modify: `src/ui/mod.rs`

**Required skill checkpoints:**
- Use `superpowers:test-driven-development` before production changes.
- Use `superpowers:requesting-code-review` after the lifecycle integration batch.
- Use `superpowers:verification-before-completion` before marking this task complete.

- [ ] **Step 1: Write the failing toggle-participation test**

Add this test, which proves the public `toggle` decision participates in the
same lifecycle transaction:

```rust
#[test]
fn toggle_waits_for_an_active_lifecycle_transaction() {
    let directory = test_state_directory("toggle-lifecycle-lock");
    let _ = std::fs::remove_dir_all(&directory);
    let store = StateStore::at(&directory);
    let worker_store = store.clone();
    let fake = support::ScriptedHerdr::start(vec![
        agent_info("w1:p1", "session-1"),
        json!({"plugin_pane":{"pane":{"pane_id":"w1:p9"}}}),
    ]);
    let socket_path = fake.socket_path().to_owned();
    let (attempting_tx, attempting_rx) = std::sync::mpsc::channel();
    let mut worker = None;

    let requests_while_locked = store
        .with_lifecycle_lock(|| {
            worker = Some(std::thread::spawn(move || {
                let client = HerdrClient::connect(socket_path).unwrap();
                attempting_tx.send(()).unwrap();
                toggle(&client, &worker_store, "w1:p1").unwrap();
            }));
            attempting_rx.recv().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            Ok(fake.requests())
        })
        .unwrap();

    worker.take().unwrap().join().unwrap();
    assert!(requests_while_locked.is_empty());
    assert_eq!(
        fake.requests()
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["agent.get", "plugin.pane.open"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}
```

- [ ] **Step 2: Run the focused test and observe RED**

Run:

```bash
cargo test --locked --test toggle_state toggle_waits_for_an_active_lifecycle_transaction -- --exact
```

Expected: the test fails because the current public `toggle` function performs
Herdr requests while the outer lifecycle transaction is still locked.

- [ ] **Step 3: Serialize the public toggle and runtime entrypoint**

Rename the existing decision body to private `toggle_unlocked`. Keep the public
API and wrap it:

```rust
pub fn toggle(client: &HerdrClient, state: &StateStore, current_pane: &str) -> AppResult<()> {
    state.with_lifecycle_lock(|| toggle_unlocked(client, state, current_pane))
}
```

Update `run_from_env` so validation and the toggle decision share one lock and
do not call the locking public wrapper recursively:

```rust
state.with_lifecycle_lock(|| {
    state.validate_saved_namespaces(&client, now_ms())?;
    toggle_unlocked(&client, &state, &current_pane)
})
```

- [ ] **Step 4: Serialize UI startup and source cleanup**

In `ui::run_from_env`, derive the agent identity and bind it while holding the
same lifecycle lock:

```rust
let identity = state_store.with_lifecycle_lock(|| {
    state_store.validate_saved_namespaces(&client, now_ms())?;
    let identity = agent_identity(&client, &source_pane)?;
    state_store.bind_verified_namespace(&source_pane, &identity.session_id, now_ms())?;
    Ok(identity)
})?;
```

When `RuntimeEvent::SourcePaneClosed` is received, run the existing cleanup in
one locked operation:

```rust
if let Err(error) = state_store
    .with_lifecycle_lock(|| state_store.remove_pane_state(&source_pane))
{
    app.transcript_error = Some(format!("source cleanup: {error}"));
}
```

- [ ] **Step 5: Run focused tests and observe GREEN**

Run:

```bash
cargo test --locked --test toggle_state toggle_waits_for_an_active_lifecycle_transaction -- --exact
cargo test --locked --test toggle_state
```

Expected: the focused regression passes and the entire toggle-state target is
green.

- [ ] **Step 6: Review and commit the integration batch**

Invoke `superpowers:requesting-code-review`, address findings within the
lifecycle integration, rerun `cargo test --locked --test toggle_state`, then
commit:

```bash
git add src/toggle.rs src/ui/mod.rs tests/toggle_state.rs
git commit -m "serialize overlay lifecycle transactions"
```

### Task 3: Verify and prepare the live plugin update

**Files:**
- Verify only; no planned source edits.

**Required skill checkpoints:**
- Tester-oriented checkpoint does not apply because this task changes no code.
- Use `superpowers:requesting-code-review` for the complete branch diff.
- Use `superpowers:verification-before-completion` before any completion claim.

- [ ] **Step 1: Run formatting and repository checks**

Run:

```bash
cargo fmt --check
git diff --check HEAD~2..HEAD
```

Expected: both commands exit successfully with no output.

- [ ] **Step 2: Run the complete test and lint gates**

Run:

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

Expected: all tests pass, Clippy reports no warnings, and the release binary is
built successfully.

- [ ] **Step 3: Review the complete change**

Invoke `superpowers:requesting-code-review` against the branch diff from its
pre-fix base. Resolve only correctness, portability, security, or regression
findings, then repeat Step 2 if code changes.

- [ ] **Step 4: Verify the installed source link safely**

After the branch is integrated according to `superpowers:finishing-a-development-branch`,
build from the linked checkout and run:

```bash
herdr plugin link /Users/samarskiy_a_s/projects/own_projects/herdr_simple_prompts
herdr plugin list --plugin herdr.simple-prompts
```

Expected: Herdr reports `herdr.simple-prompts` from the repository source path.
Do not invoke the toggle action globally during verification because it can
change whichever Herdr pane is currently focused.
