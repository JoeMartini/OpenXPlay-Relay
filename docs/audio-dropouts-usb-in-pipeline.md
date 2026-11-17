# Audio dropouts on Linux wired CarPlay: the USB IN endpoint is starved

**Symptom.** Music played through the relay cuts out for hundreds of milliseconds — occasionally
for a second or more — several times a minute. The relay's own receive statistics show it plainly:

```
Receive: audio type=100 packets=678 bytes=… seqForwardGaps=39 seqGapEvents=3 seqGapMax=39
                              interArrivalMaxMs=763 readMaxMs=763
```

Roughly 2–3 % of audio RTP packets never arrive, in bursts of 30–55 packets (250–420 ms each).

**Why it is easy to misdiagnose.** Every host-side counter a Linux engineer would normally check
is clean, because the packets are not lost on the host at all:

| Check | Result while packets were being lost |
|---|---|
| TAP interface (`rx_dropped`, `rx_errors`, `rx_missed_errors`, `rx_fifo_errors`) | all 0 |
| `Udp: RcvbufErrors` on the kernel | no growth |
| Socket receive buffer raised to 512 KiB (via `net.core.rmem_max`) | loss unchanged (3.7 % → 3.72 %) |
| NCM frame decoder resyncs / skipped bytes | 0 |
| RTP sequence analysis on the wire (pcap on the TAP) | gaps present, **0 reordering, 0 duplicates** |
| Video stream | **no loss** — it is TCP, so retransmission hides it |

So "all host counters are clean" does **not** mean "the host did not drop anything". Here the
packets were destroyed inside the iPhone's NCM uplink FIFO, because the host had no IN request
posted while it was busy consuming data.

## Root cause

`carplay-iap2/src/transport/linux_usb_host.rs` submitted **exactly one URB per IN endpoint**, and
`input()` returned buffered data *without re-submitting* the URB:

```rust
struct Pending {
    urb: Box<Urb>,
    buffer: Vec<u8>,
    queued: bool,
    ready: VecDeque<Vec<u8>>,
    failure: Option<i32>,
}
```

Once the single URB completed, its bytes sat in `ready`; the endpoint had nothing queued until the
consumer came back for more data. During that window (frame decoding, TAP write, lock contention)
the device had no place to put incoming Ethernet frames and dropped them itself. Audio is UDP, so
those packets are gone forever; video is TCP, so it silently retransmitted.

The same class of problem is documented by other CarPlay relay projects ("the USB link no longer
drops packets, which caused CarPlay audio dropouts").

## Fix

Keep a small pipeline of URBs in flight per endpoint and refill it as soon as one completes:

- `Pending` holds `IN_URBS = 4` slots, each with its own URB and buffer
  (`max_bytes.min(65536)`, i.e. 32 KiB for NCM bulk IN);
- `reap()` copies the completed slot's bytes into `ready` and **immediately re-submits** that slot;
- `input()` refills idle slots *before* handing data to the caller, so the endpoint is never idle;
- `close()` discards every queued slot, and reaping stops re-submitting once the device is closed
  (otherwise the cancel path never drains and times out).

Ordering is preserved: slots are marked idle only after `reap()`, completions are handled in kernel
completion order, and the existing `split_off` tail-requeue still handles `max_bytes` slicing.
Memory cost is 4 × 32 KiB ≈ 128 KiB per endpoint.

### Test semantics changed on purpose

The upstream test `timeout_keeps_one_urb_and_next_reap_keeps_every_byte` encoded the
one-URB-in-flight contract as intended behaviour. It is renamed to
`endpoint_keeps_in_urbs_posted_and_next_reap_keeps_every_byte` and asserts the new contract — the
parts that matter to callers (no bytes lost when a buffer is sliced, a timeout never drops the URB)
are unchanged. Anything that rebases on upstream must redo this test, not merge it mechanically.

## Measured result

Same measurement method before and after (RTP sequence numbers captured on the TAP interface;
sequence span minus the number of distinct sequence numbers = truly missing packets):

| Metric | Before | After |
|---|---|---|
| 4 × 20 s captures | 3.72 % missing (10,141 packets / 377 missing) | **0.00 %** (10,554 packets / 0 missing) |
| Relay log, 5 s windows | 84 windows / 56,006 packets / 1,213 gaps = 2.17 % | **416 windows / 282,178 packets / 0 gaps** |
| Worst window | 55 packets missing | 0 |
| `interArrivalMaxMs` | 241–763 ms | 20–153 ms |

A 34-minute continuous playback produced zero sequence gaps, so the effect is not sample noise.

## Diagnostic recipe for the next time

1. Establish an objective metric first: gap rate (`seqForwardGaps / packets`) and burst-length
   distribution, not "it sounds bad".
2. Check **in-flight request counts and queue depths at both ends** (URBs, async reads) before
   touching buffer sizes.
3. Treat "host counters are clean" as *inconclusive*, never as proof the host is innocent.
4. If UDP suffers while TCP does not, suspect link-level loss before blaming the decoder.
