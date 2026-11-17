# tools/

Operational glue that makes the relay's supervision honest. Nothing here is required to build
the relay; it exists because a relay that is *down* and a relay that is *up but unreachable*
look identical to a supervisor that only checks whether a process exists.

- `relay-restart.sh` — the single restart entry point. Kills every relay process and waits for
  it to be gone, clears a stale stream socket, launches, then polls the receiver URL until it
  answers HTTP 200 (one clean-up-and-retry). Returns 0 only when the endpoint is actually
  serving, 5 otherwise, and prints the tail of the log on failure.
- `relay-watchdog.py` — readiness-gated supervisor. Probes the receiver URL; restarts through
  `relay-restart.sh` when it does not answer; counts **only successful restarts** against the
  hourly budget; backs off after a failed restart; writes JSONL events. `--dry-run` and
  `--self-test` are safe (no restarts, no network).

Why this matters: with the previous supervisor, three restarts that never came up consumed the
whole budget and the relay stayed down for eighteen minutes, which the user experienced as an
HTTP 502 from the front proxy. The measurement that matters is the endpoint the user depends on,
not process liveness or log freshness.

Both scripts are environment-configurable (`OXP_*`), so they can be exercised against fake
processes and a local HTTP server without touching a live relay:

```sh
OXP_PATTERN=fakerelayproc \
OXP_LAUNCH=/tmp/fake-launch.sh \
OXP_READY_URL=http://127.0.0.1:18999/ \
OXP_SOCK=/tmp/fake.sock \
./tools/relay-restart.sh    # exit 0 with "ready"; exit 5 if nothing comes up
```
