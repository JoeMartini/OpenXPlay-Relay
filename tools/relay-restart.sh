#!/usr/bin/env bash
# Restart the relay and report success only when the receiver endpoint actually answers.
#
# Why this exists: a supervisor that relaunches the process without verifying readiness will
# happily burn its retry budget on restarts that never came up (for example when the old process
# still holds the state directory, so the new one exits immediately), and then stop retrying
# exactly when it is needed — leaving the user with a dead page (HTTP 502 from the front proxy)
# and no process at all.
#
# Everything is overridable by environment variable so this can be exercised against fakes:
#   OXP_BASE            deployment root (default: parent of this script's directory)
#   OXP_LAUNCH          launcher command (default: $OXP_BASE/scripts/relay-launch.sh)
#   OXP_PATTERN         pgrep -f pattern matching the relay processes
#   OXP_SOCK            unix socket the launcher creates; stale copies are removed
#   OXP_READY_URL       URL polled for readiness (HTTP 200 = ready)
#   OXP_LOG             file to append launcher output to
#   OXP_READY_TIMEOUT   seconds to wait for readiness per attempt (default 60)
#   OXP_START_GRACE     seconds before "no process" is treated as a failed start (default 8)
#
# Exit codes: 0 ready, 5 not ready after two attempts.
set -uo pipefail

BASE=${OXP_BASE:-$(cd "$(dirname "$0")/.." && pwd)}
LAUNCH=${OXP_LAUNCH:-$BASE/scripts/relay-launch.sh}
PAT=${OXP_PATTERN:-target/release/(openxplay-relay|oxp-relay)}
SOCK=${OXP_SOCK:-/tmp/oxp-relay.sock}
READY_URL=${OXP_READY_URL:-https://127.0.0.1:18444/}
LOG=${OXP_LOG:-/dev/null}
READY_TIMEOUT=${OXP_READY_TIMEOUT:-60}
START_GRACE=${OXP_START_GRACE:-8}

pids() { pgrep -f "$PAT" 2>/dev/null | grep -v "^$$\$" || true; }

stop() {
  local listed
  listed=$(pids)
  [ -z "$listed" ] && return 0
  # shellcheck disable=SC2086
  kill $listed 2>/dev/null || true
  for _ in $(seq 1 30); do [ -z "$(pids)" ] && return 0; sleep 0.5; done
  listed=$(pids)
  # shellcheck disable=SC2086
  kill -9 $listed 2>/dev/null || true
  for _ in $(seq 1 10); do [ -z "$(pids)" ] && break; sleep 0.5; done
}

ready() {
  local code
  code=$(curl -sk -o /dev/null -w '%{http_code}' --max-time 4 "$READY_URL" 2>/dev/null || true)
  [ "$code" = "200" ]
}

attempt() {
  # A leftover socket makes a fresh process believe its peer is still alive.
  [ -e "$SOCK" ] && { rm -f "$SOCK" 2>/dev/null || sudo -n rm -f "$SOCK" 2>/dev/null || true; }
  local started=$(( $(date +%s) + START_GRACE ))
  setsid "$LAUNCH" >>"$LOG" 2>&1 &
  local deadline=$(( $(date +%s) + READY_TIMEOUT ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    ready && return 0
    # After the grace period, no process at all means it started and died immediately.
    if [ "$(date +%s)" -gt "$started" ] && [ -z "$(pids)" ]; then return 1; fi
    sleep 1
  done
  return 1
}

stop
if attempt; then echo "ready"; exit 0; fi
echo "first attempt not ready; cleaning up and retrying" >&2
stop
if attempt; then echo "ready-after-retry"; exit 0; fi

echo "failed: $READY_URL did not return 200 within ${READY_TIMEOUT}s. Recent log:" >&2
tail -c 6000 "$LOG" 2>/dev/null | tail -6 >&2
exit 5
