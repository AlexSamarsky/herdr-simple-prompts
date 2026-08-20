# Tested agent versions design

## Context

The public README currently says only that Simple Prompts supports Codex CLI
and Claude Code. That wording does not distinguish versions exercised against
the plugin from future agent releases whose transcript or terminal layout may
change.

The compatibility statement must reflect the approved evidence boundary:

- Codex CLI `0.146.0` through `0.148.0`;
- Claude Code `2.1.237`;
- releases newer than those upper bounds have not been verified.

## Decision

Keep compatibility information in the public README, next to the existing
version-0.1 support statement and installation prerequisites. Replace the
generic support sentence with a compact table whose columns are agent and
tested versions. Follow it with one explicit sentence that newer agent releases
may work but have not yet been verified.

Update the prerequisite sentence to name the same tested versions. Do not use
an open-ended `0.146+` claim because that would imply verified compatibility
with future Codex releases.

## Scope

This is a documentation-only clarification. It does not change the parser,
runtime behavior, plugin manifest, dependency versions, or minimum Herdr and
Rust versions. Detailed behavior documentation remains unchanged so the tested
version boundary has one public source of truth.

## Verification

Review the README diff for consistency and run the repository's required
formatting, Clippy, all-target test, and locked release-build gates before the
authorized push.
