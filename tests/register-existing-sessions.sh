#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/simple-prompts-session-test.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

fake_bin="$fixture_root/bin"
surface_root="$fixture_root/surfaces"
sessions_root="$fixture_root/codex/sessions"
agents_file="$fixture_root/agents.json"
report_log="$fixture_root/report.log"
output_log="$fixture_root/output.log"
primary_id="11111111-1111-4111-8111-111111111111"
secondary_id="22222222-2222-4222-8222-222222222222"

mkdir -p "$fake_bin" "$surface_root" "$sessions_root"

cat >"$fake_bin/herdr" <<'FAKE_HERDR'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-} ${2:-}" in
  "agent list")
    cat "$FAKE_AGENT_LIST"
    ;;
  "agent read")
    cat "$FAKE_SURFACE_ROOT/${3}.txt"
    ;;
  "pane report-agent-session")
    printf '%s\n' "$*" >>"$FAKE_REPORT_LOG"
    if [ "${FAKE_REPORT_FAIL:-0}" -eq 1 ]; then
      printf 'synthetic report failure\n' >&2
      exit 1
    fi
    ;;
  *)
    printf 'unexpected fake herdr command\n' >&2
    exit 64
    ;;
esac
FAKE_HERDR
chmod +x "$fake_bin/herdr"

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

reset_case() {
  rm -rf "$surface_root" "$sessions_root"
  mkdir -p "$surface_root" "$sessions_root"
  : >"$report_log"
  : >"$output_log"
  unset FAKE_REPORT_FAIL
}

write_agents() {
  printf '%s\n' "$1" >"$agents_file"
}

write_surface() {
  local pane="$1"
  local content="$2"
  printf '%b\n' "$content" >"$surface_root/$pane.txt"
}

add_transcript() {
  local directory="$1"
  local filename="$2"
  mkdir -p "$sessions_root/$directory"
  : >"$sessions_root/$directory/$filename"
}

run_helper() {
  set +e
  PATH="$fake_bin:$PATH" \
    HERDR_BIN="$fake_bin/herdr" \
    FAKE_AGENT_LIST="$agents_file" \
    FAKE_SURFACE_ROOT="$surface_root" \
    FAKE_REPORT_LOG="$report_log" \
    FAKE_REPORT_FAIL="${FAKE_REPORT_FAIL:-0}" \
    CODEX_SESSIONS_ROOT="$sessions_root" \
    bash "$repo_root/scripts/register-existing-sessions.sh" \
      >"$output_log" 2>&1
  RUN_STATUS=$?
  set -e
}

assert_status() {
  local expected="$1"
  [ "$RUN_STATUS" -eq "$expected" ] || {
    sed -n '1,120p' "$output_log" >&2
    fail "expected status $expected, got $RUN_STATUS"
  }
}

assert_no_session_ids_in_output() {
  if rg -q "$primary_id|$secondary_id" "$output_log"; then
    sed -n '1,120p' "$output_log" >&2
    fail "helper leaked a session id"
  fi
}

unregistered_agent_json() {
  local agent="$1"
  printf '{"result":{"agents":[{"pane_id":"w1:p1","agent":"%s","agent_session":null}]}}' \
    "$agent"
}

reset_case
write_agents "$(unregistered_agent_json codex)"
write_surface "w1:p1" "gpt-5.6-sol xhigh · /repo · weekly 47% left · 12.3M used · $primary_id"
add_transcript "2026/08/14" "rollout-2026-08-14T10-00-00-$primary_id.jsonl"
run_helper
assert_status 0
[ "$(wc -l <"$report_log" | tr -d ' ')" -eq 1 ] || fail "expected one report"
rg -q -- '--agent codex' "$report_log" || fail "report omitted agent kind"
rg -q -- '--session-start-source resume' "$report_log" || fail "report omitted resume source"
rg -q -- "--agent-session-id $primary_id" "$report_log" || fail "report omitted recovered id"
rg -q 'Registered w1:p1' "$output_log" || fail "success message omitted pane"
assert_no_session_ids_in_output

reset_case
write_agents "{\"result\":{\"agents\":[{\"pane_id\":\"w1:p1\",\"agent\":\"codex\",\"agent_session\":{\"kind\":\"id\",\"agent\":\"codex\",\"value\":\"$primary_id\"}}]}}"
run_helper
assert_status 0
[ ! -s "$report_log" ] || fail "already registered pane was reported again"
rg -q 'No unregistered Codex or Claude panes found' "$output_log" || fail "missing no-op message"
assert_no_session_ids_in_output

reset_case
write_agents "$(unregistered_agent_json codex)"
write_surface "w1:p1" "gpt-5.6-sol xhigh · /quoted · weekly 47% left · 12.3M used · $primary_id\n• That footer-shaped line was conversation text.\ngpt-5.6-sol xhigh · /repo · weekly 47% left"
add_transcript "one" "$primary_id.jsonl"
run_helper
assert_status 1
[ ! -s "$report_log" ] || fail "footer-shaped conversation text was reported"
rg -q 'expected one native session candidate, found 0' "$output_log" || fail "missing zero-candidate reason"
assert_no_session_ids_in_output

reset_case
write_agents "$(unregistered_agent_json codex)"
write_surface "w1:p1" "gpt-5.6-sol xhigh · /repo · weekly 47% left · $primary_id · $secondary_id"
add_transcript "one" "$primary_id.jsonl"
add_transcript "two" "$secondary_id.jsonl"
run_helper
assert_status 1
[ ! -s "$report_log" ] || fail "multiple footer candidates were reported"
rg -q 'expected one native session candidate, found 2' "$output_log" || fail "missing multiple-candidate reason"
assert_no_session_ids_in_output

reset_case
write_agents "$(unregistered_agent_json codex)"
write_surface "w1:p1" "gpt-5.6-sol xhigh · /repo · weekly 47% left · $primary_id"
add_transcript "one" "$primary_id.jsonl"
add_transcript "two" "rollout-$primary_id.jsonl"
run_helper
assert_status 1
[ ! -s "$report_log" ] || fail "ambiguous transcript was reported"
rg -q 'expected one matching Codex transcript, found 2' "$output_log" || fail "missing transcript ambiguity reason"
assert_no_session_ids_in_output

reset_case
write_agents "$(unregistered_agent_json claude)"
run_helper
assert_status 1
[ ! -s "$report_log" ] || fail "Claude identity was guessed"
rg -q 'restart or resume Claude' "$output_log" || fail "missing Claude recovery guidance"
assert_no_session_ids_in_output

reset_case
write_agents "$(unregistered_agent_json codex)"
write_surface "w1:p1" "gpt-5.6-sol xhigh · /repo · weekly 47% left · $primary_id"
add_transcript "one" "$primary_id.jsonl"
export FAKE_REPORT_FAIL=1
run_helper
assert_status 1
rg -q 'Herdr rejected the session report' "$output_log" || fail "missing report failure reason"
assert_no_session_ids_in_output

printf 'existing-session helper tests passed\n'
