#!/usr/bin/env bash
set -eu

pane="${HERDR_PANE_ID:-}"
if [ -z "$pane" ]; then
  printf 'herdr-simple-prompts: toggle: HERDR_PANE_ID is not set\n' >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
recovery_script="${SIMPLE_PROMPTS_RECOVERY_SCRIPT:-$script_dir/register-existing-sessions.sh}"
toggle_bin="${SIMPLE_PROMPTS_TOGGLE_BIN:-$script_dir/../target/release/herdr-simple-prompts}"

bash "$recovery_script" --pane "$pane"
exec "$toggle_bin" toggle
