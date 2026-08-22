#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/simple-prompts-toggle-test.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT
call_log="$fixture_root/calls.log"
output_log="$fixture_root/output.log"

cat >"$fixture_root/recover.sh" <<'FAKE_RECOVERY'
#!/usr/bin/env bash
printf 'recover:%s\n' "$*" >>"$CALL_LOG"
exit "${RECOVERY_STATUS:-0}"
FAKE_RECOVERY

cat >"$fixture_root/toggle" <<'FAKE_TOGGLE'
#!/usr/bin/env bash
printf 'toggle:%s\n' "$*" >>"$CALL_LOG"
exit "${TOGGLE_STATUS:-0}"
FAKE_TOGGLE
chmod +x "$fixture_root/recover.sh" "$fixture_root/toggle"

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  sed -n '1,120p' "$output_log" >&2
  exit 1
}

run_wrapper() {
  local pane="$1"
  local recovery_status="$2"
  local toggle_status="$3"
  : >"$call_log"
  : >"$output_log"

  set +e
  if [ "$pane" = "__unset__" ]; then
    env -u HERDR_PANE_ID \
      SIMPLE_PROMPTS_RECOVERY_SCRIPT="$fixture_root/recover.sh" \
      SIMPLE_PROMPTS_TOGGLE_BIN="$fixture_root/toggle" \
      CALL_LOG="$call_log" \
      RECOVERY_STATUS="$recovery_status" \
      TOGGLE_STATUS="$toggle_status" \
      bash "$repo_root/scripts/toggle-with-session-recovery.sh" \
        >"$output_log" 2>&1
  else
    HERDR_PANE_ID="$pane" \
      SIMPLE_PROMPTS_RECOVERY_SCRIPT="$fixture_root/recover.sh" \
      SIMPLE_PROMPTS_TOGGLE_BIN="$fixture_root/toggle" \
      CALL_LOG="$call_log" \
      RECOVERY_STATUS="$recovery_status" \
      TOGGLE_STATUS="$toggle_status" \
      bash "$repo_root/scripts/toggle-with-session-recovery.sh" \
        >"$output_log" 2>&1
  fi
  RUN_STATUS=$?
  set -e
}

assert_status() {
  local expected="$1"
  [ "$RUN_STATUS" -eq "$expected" ] || fail "expected status $expected, got $RUN_STATUS"
}

run_wrapper "w1:p1" 0 0
assert_status 0
[ "$(sed -n '1p' "$call_log")" = "recover:--pane w1:p1" ] || fail "recovery did not run first"
[ "$(sed -n '2p' "$call_log")" = "toggle:toggle" ] || fail "toggle did not run second"
[ "$(wc -l <"$call_log" | tr -d ' ')" -eq 2 ] || fail "success path made extra calls"

run_wrapper "w1:p1" 7 0
assert_status 7
[ "$(cat "$call_log")" = "recover:--pane w1:p1" ] || fail "recovery failure still ran toggle"

run_wrapper "w1:p1" 0 9
assert_status 9
[ "$(sed -n '1p' "$call_log")" = "recover:--pane w1:p1" ] || fail "toggle failure skipped recovery"
[ "$(sed -n '2p' "$call_log")" = "toggle:toggle" ] || fail "toggle failure did not preserve its call"
[ "$(wc -l <"$call_log" | tr -d ' ')" -eq 2 ] || fail "toggle failure made extra calls"

run_wrapper "__unset__" 0 0
assert_status 2
[ ! -s "$call_log" ] || fail "missing pane context ran a child command"
rg -q 'HERDR_PANE_ID' "$output_log" || fail "missing pane diagnostic omitted HERDR_PANE_ID"

printf 'toggle recovery wrapper tests passed\n'
