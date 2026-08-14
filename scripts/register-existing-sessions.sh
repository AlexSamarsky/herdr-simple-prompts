#!/usr/bin/env bash
set -u

herdr_bin="${HERDR_BIN:-herdr}"
jq_bin="${JQ_BIN:-jq}"
rg_bin="${RG_BIN:-rg}"
sessions_root="${CODEX_SESSIONS_ROOT:-${CODEX_HOME:-$HOME/.codex}/sessions}"
uuid_pattern='[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
footer_pattern="^[[:space:]]*gpt-[A-Za-z0-9._ -]+[[:space:]]*·[[:space:]]*(~|/)[^·]*[[:space:]]*·.*[[:space:]]·[[:space:]]*${uuid_pattern}([[:space:]]*·[^·]+)*[[:space:]]*$"

for required_command in "$herdr_bin" "$jq_bin" "$rg_bin"; do
  if ! command -v "$required_command" >/dev/null 2>&1; then
    printf 'Missing required command: %s\n' "$required_command" >&2
    exit 2
  fi
done

if ! agents_json="$("$herdr_bin" agent list 2>/dev/null)"; then
  printf 'Unable to read Herdr agents.\n' >&2
  exit 1
fi

if ! unregistered="$(
  printf '%s' "$agents_json" |
    "$jq_bin" -r '
      .result.agents[]
      | select(.agent == "codex" or .agent == "claude")
      | select((.agent_session.kind? // "") != "id")
      | [.pane_id, .agent]
      | @tsv
    ' 2>/dev/null
)"; then
  printf 'Unable to parse Herdr agents.\n' >&2
  exit 1
fi

failed=0
seen=0
while IFS=$'\t' read -r pane agent; do
  [ -n "$pane" ] || continue
  seen=1

  if [ "$agent" = "claude" ]; then
    printf 'Skipped %s: restart or resume Claude after installing the Herdr integration.\n' \
      "$pane" >&2
    failed=1
    continue
  fi

  if ! surface="$("$herdr_bin" agent read "$pane" --source visible --lines 12 --format text 2>/dev/null)"; then
    printf 'Skipped %s: unable to read the native footer.\n' "$pane" >&2
    failed=1
    continue
  fi

  footer_line="$(printf '%s\n' "$surface" | sed '/^[[:space:]]*$/d' | tail -n 1)"
  candidates="$(
    printf '%s\n' "$footer_line" |
      "$rg_bin" "$footer_pattern" 2>/dev/null |
      "$rg_bin" -o "$uuid_pattern" 2>/dev/null |
      tr '[:upper:]' '[:lower:]' |
      sort -u
  )"
  candidate_count="$(printf '%s\n' "$candidates" | sed '/^$/d' | wc -l | tr -d ' ')"
  if [ "$candidate_count" -ne 1 ]; then
    printf 'Skipped %s: expected one native session candidate, found %s.\n' \
      "$pane" "$candidate_count" >&2
    failed=1
    continue
  fi
  candidate="$(printf '%s\n' "$candidates" | sed -n '1p')"

  matches="$(
    find "$sessions_root" -type f \
      \( -name "$candidate.jsonl" -o -name "*-$candidate.jsonl" \) \
      -print 2>/dev/null
  )"
  match_count="$(printf '%s\n' "$matches" | sed '/^$/d' | wc -l | tr -d ' ')"
  if [ "$match_count" -ne 1 ]; then
    printf 'Skipped %s: expected one matching Codex transcript, found %s.\n' \
      "$pane" "$match_count" >&2
    failed=1
    continue
  fi

  if "$herdr_bin" pane report-agent-session "$pane" \
    --source herdr:simple-prompts-existing-sessions \
    --agent codex \
    --agent-session-id "$candidate" \
    --session-start-source resume >/dev/null 2>&1; then
    printf 'Registered %s.\n' "$pane"
  else
    printf 'Skipped %s: Herdr rejected the session report.\n' "$pane" >&2
    failed=1
  fi
done <<<"$unregistered"

if [ "$seen" -eq 0 ]; then
  printf 'No unregistered Codex or Claude panes found.\n'
fi

exit "$failed"
