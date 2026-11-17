#!/usr/bin/env python3
"""Readiness-gated watchdog for the relay.

Design rule this encodes: a supervisor's job is not "restart things", it is "know whether the
restart worked". Restarts that never came up must not consume the retry budget, and a stuck
restart must be retried with backoff instead of hammering every tick.

Health is decided by probing the endpoint the user actually depends on (HTTP 200), not by
looking at processes or log freshness alone: a relay can be running, logging, even reporting an
active session while the receiver socket is dead — which is exactly the state that produced a
502 to the user.

Configuration (environment):
  OXP_READY_URL        URL that must answer 200 (default https://127.0.0.1:18444/)
  OXP_RESTART_CMD      command that restarts and verifies readiness; exit 0 = success
                       (default: <repo>/tools/relay-restart.sh)
  OXP_EVENTS           JSONL event log (default ./relay-watchdog.jsonl)
  OXP_BUDGET           successful restarts allowed per window (default 3)
  OXP_BUDGET_WINDOW    window in seconds (default 3600)
  OXP_BACKOFF          seconds to wait after a failed restart (default 600)
  OXP_PROBE            set to 0 to skip the HTTP probe (offline tests)

Usage: relay-watchdog.py [--dry-run] [--self-test]
Exit codes: 0 nothing to do / healthy, 2 restart attempted but failed, 1 budget exhausted.
"""

import json
import os
import subprocess
import sys
import time

BASE = os.environ.get("OXP_BASE", os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
READY_URL = os.environ.get("OXP_READY_URL", "https://127.0.0.1:18444/")
RESTART_CMD = os.environ.get("OXP_RESTART_CMD", os.path.join(BASE, "tools", "relay-restart.sh"))
EVENTS = os.environ.get("OXP_EVENTS", os.path.join(BASE, "relay-watchdog.jsonl"))
BUDGET = int(os.environ.get("OXP_BUDGET", "3"))
BUDGET_WINDOW = float(os.environ.get("OXP_BUDGET_WINDOW", "3600"))
BACKOFF = float(os.environ.get("OXP_BACKOFF", "600"))


def web_ready() -> bool:
    if os.environ.get("OXP_PROBE", "1") == "0":
        return True
    try:
        code = subprocess.run(
            ["curl", "-sk", "-o", "/dev/null", "-w", "%{http_code}", "--max-time", "4", READY_URL],
            capture_output=True, text=True, timeout=8,
        ).stdout.strip()
    except Exception:
        return False
    return code == "200"


def append_event(payload: dict) -> None:
    payload = {"time": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), **payload}
    directory = os.path.dirname(EVENTS)
    if directory:
        os.makedirs(directory, exist_ok=True)
    with open(EVENTS, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(payload) + "\n")


def recent(kind: str, now: float, window: float) -> int:
    if not os.path.exists(EVENTS):
        return 0
    count = 0
    with open(EVENTS, encoding="utf-8") as handle:
        for line in handle:
            try:
                item = json.loads(line)
                stamp = time.mktime(time.strptime(item["time"], "%Y-%m-%dT%H:%M:%SZ"))
            except Exception:
                continue
            if item.get("kind") == kind and now - stamp <= window:
                count += 1
    return count


def restart(state: dict) -> tuple:
    try:
        result = subprocess.run([RESTART_CMD], capture_output=True, text=True, timeout=300)
        ok = result.returncode == 0
        detail = ((result.stdout or "") + (result.stderr or "")).strip()[-400:]
    except Exception as error:
        ok, detail = False, f"{type(error).__name__}: {error}"
    append_event({"kind": "restart" if ok else "restart-failed", "detail": detail, "state": state})
    return ok, detail


def main() -> int:
    dry = "--dry-run" in sys.argv
    now = time.time()
    ready = web_ready()
    state = {"ready_url": READY_URL, "web": "ready" if ready else "not-ready"}
    if ready:
        if dry:
            print("healthy", state)
        return 0

    if recent("restart-failed", now, BACKOFF) > 0:
        if dry:
            print("backoff-after-failed-restart", state)
        return 0

    used = recent("restart", now, BUDGET_WINDOW)
    if used >= BUDGET:
        if recent("giveup", now, BUDGET_WINDOW) == 0:
            append_event({"kind": "giveup", "restarts_in_window": used, "state": state})
        if dry:
            print("giveup", state)
        return 1

    if dry:
        print("would-restart", state)
        return 0

    ok, detail = restart(state)
    print(f"restart ok={ok} detail={detail[-160:]!r}", state)
    return 0 if ok else 2


def self_test() -> int:
    global EVENTS, web_ready
    os.environ["OXP_EVENTS"] = os.path.join(os.path.dirname(__file__) or ".", "relay-watchdog.selftest.jsonl")
    EVENTS = os.environ["OXP_EVENTS"]
    if os.path.exists(EVENTS):
        os.remove(EVENTS)
    web_ready = lambda: False          # 自测里固定当作"接收端不可用"，且不联网
    checks = []
    # Backoff wins over restart, and the budget only counts successful restarts.
    globals()["recent"] = lambda kind, now, window: {"restart-failed": 1}.get(kind, 0)
    checks.append(("backoff after a failed restart", main() == 0))
    globals()["recent"] = lambda kind, now, window: {"restart": BUDGET}.get(kind, 0)
    checks.append(("budget exhaustion gives up", main() == 1))
    globals()["recent"] = lambda kind, now, window: {"restart": BUDGET - 1}.get(kind, 0)
    globals()["restart"] = lambda state: (True, "stub")        # 自测绝不真的重启
    checks.append(("budget remaining allows a restart", main() == 0))
    globals()["restart"] = lambda state: (False, "stub failure")
    checks.append(("failed restart is reported", main() == 2))

    if os.path.exists(EVENTS):
        os.remove(EVENTS)
    ok = True
    for label, passed in checks:
        print(f"  {'OK' if passed else 'FAIL'} {label}")
        ok = ok and passed
    print("self-test:", "passed" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(self_test() if "--self-test" in sys.argv else main())
