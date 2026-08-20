# Tested Agent Versions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Before coding, invoke a tester-oriented skill. After each meaningful coding batch, invoke superpowers:requesting-code-review. Before any completion claim, invoke superpowers:verification-before-completion. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish an exact, non-open-ended statement of the Codex CLI and Claude Code versions verified with Simple Prompts.

**Architecture:** Keep the compatibility boundary in the public README as its single human-facing source of truth. Replace the generic agent-support sentence and prerequisite wording with the approved tested range, while explicitly withholding claims about newer releases.

**Tech Stack:** Markdown, Git, Cargo verification gates

---

### Task 1: Document the tested agent versions

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/plans/2026-08-20-tested-agent-versions.md`

**Required skill checkpoints:**
- The tester-oriented checkpoint does not apply because this task changes documentation only and does not alter executable behavior.
- Invoke `superpowers:requesting-code-review` after the README batch.
- Invoke `superpowers:verification-before-completion` before reporting or publishing the result.

- [x] **Step 1: Replace the generic support sentence with the tested matrix**

Replace:

```markdown
Version 0.1 supports Codex CLI and Claude Code on macOS and Linux.
```

with:

```markdown
Version 0.1 has been tested with these agent versions on macOS and Linux:

| Agent | Tested versions |
|---|---|
| Codex CLI | `0.146.0`–`0.148.0` |
| Claude Code | `2.1.237` |

Newer agent releases may work, but have not yet been verified.
```

- [x] **Step 2: Align the installation prerequisite**

Replace:

```markdown
You need Herdr 0.7.5+, Rust 1.88+ with Cargo, and Codex CLI or Claude Code.
```

with:

```markdown
You need Herdr 0.7.5+, Rust 1.88+ with Cargo, and either Codex CLI
`0.146.0`–`0.148.0` or Claude Code `2.1.237`.
```

- [x] **Step 3: Review the public wording**

Run:

```bash
git diff --check
rg -n "0\\.146|0\\.148|2\\.1\\.237|newer agent releases" README.md
```

Expected: no whitespace errors; the tested bounds appear only in the support matrix and aligned prerequisite, with one explicit newer-version caveat.

- [x] **Step 4: Review and commit the documentation batch**

Invoke `superpowers:requesting-code-review`, address concrete findings, then commit only the approved documentation files:

```bash
git add -- README.md docs/superpowers/plans/2026-08-20-tested-agent-versions.md
git commit -m "document tested agent versions"
```

### Task 2: Verify and publish the authorized main update

**Files:**
- Verify only: repository tree

**Required skill checkpoints:**
- The tester-oriented checkpoint does not apply because Task 1 changes documentation only.
- Invoke `superpowers:verification-before-completion` before the push and final report.

- [ ] **Step 1: Run the repository gates**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --locked --release
```

Expected: formatting and Clippy succeed, all tests pass, and the locked release build exits zero.

- [ ] **Step 2: Fast-forward the local main branch**

Run from the repository root:

```bash
git switch main
git merge --ff-only docs/tested-agent-versions
```

Expected: `main` advances to the reviewed documentation commit without a merge commit.

- [ ] **Step 3: Confirm the exact publish scope**

Run:

```bash
git status --short
git log --oneline origin/main..main
git diff --check origin/main..main
```

Expected: the worktree is clean, only the reviewed local commits are ahead of `origin/main`, and the complete publish diff has no whitespace errors.

- [ ] **Step 4: Push main without force and verify synchronization**

Run:

```bash
git push origin main
git rev-list --left-right --count origin/main...main
```

Expected: the push succeeds and the final ahead/behind counts are `0 0`.
