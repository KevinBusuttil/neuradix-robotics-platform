# RFC-0018 — Python Worker SDK and Process Isolation

- Status: Partially implemented (integrated WP-A05 I/O/heartbeat; per-process Linux resource branch)
- Current plan: [Implementation Plan v0.4, WP-A05](../Neuradix_Implementation_Plan_v0.4.md#wp-a05); original scope: Functional Specification v0.5 §19, §41.6; complements RFC-0017
- Crate: `neuradix-python`; Python library: `python/neuradix_worker.py`

## Problem

Python must be first-class for AI/analysis but isolated from safety-critical
paths (§3.7, §19.4). The defining requirement is crash safety (§41.6): a Python
component must be able to crash **without terminating control and safety
processes**. Python must also stay out of the deterministic control path
(EXEC-007).

## Scope

Implemented in this increment: a Rust-side supervisor that runs a Python
component as an **isolated OS process** over a line-delimited JSON protocol, with
responsiveness health, bounded stdio and total deadlines, cleanup and restart
attempt budgets; and a Python `run(handler)` library. Linux GNU/musl is the only
implemented bounded backend; other OSes reject launch explicitly.
Out of scope for this increment:
in-process PyO3/Maturin bindings and NumPy zero-copy views (§19.1–§19.2),
content-addressed/locked dependency environments, aggregate CPU/RSS and GPU limits,
wheel packaging, escaped-session containment and additional OS backends.
See [WP-A05 design and evidence](../implementation/WP-A05-Bounded-Worker-IO.md).
The [heartbeat policy and evidence](../implementation/WP-A05-Worker-Heartbeat.md)
defines the subsequent bounded increment and its required scheduling.
The [resource policy and evidence](../implementation/WP-A05-Worker-Resource-Limits.md)
adds Linux RLIMIT_CPU/RLIMIT_AS enforcement before Python executes.

## Proposed decision

### Isolation model

A Python component runs as a separate process (trusted native launcher, then
`exec python3 <script>` in the same PID/process group). It never
shares the runtime's address space, so a Python exception or process crash does
not directly corrupt the runtime's memory. This is not a security sandbox:
same-user privileges, comprehensive resource containment and escaped-session
cleanup are not qualified by process separation.

### Protocol (newline-delimited JSON)

Before the worker protocol, a trusted launcher confirms `limits-v1` with the
effective CPU/AS values, then consumes a fixed `start` acknowledgement. The parent
checks the values before acknowledging. Bootstrap and ready share the original
handshake deadline and 64-byte/one-message minimum limits. Later worker-provided
setup claims never become trusted resource evidence.

- startup handshake: worker → `{"kind":"ready","name","skipPolicy"}`;
- request: supervisor → `{"kind":"request","seq","payload"}`;
- response: worker → `{"kind":"response","seq","payload"}` or
  `{"kind":"error","seq","message"}`;
- heartbeat: `{"kind":"ping","seq":N}` → `{"kind":"pong","seq":N}`;
  the old response/payload/pong shape no longer satisfies a heartbeat;
- `{"kind":"shutdown"}` ends processing.
- stdout carries only protocol JSON; stderr remains inherited for diagnostics.
  EOF does not prove process exit: live-child EOF is an explicit terminal error.
- incoming/outgoing line limits count UTF-8 bytes including newline; bounded
  queue bytes include complete and partial lines. Overload rejects explicitly.
- one request is outstanding; sequence starts at 1 and never wraps. Unrelated
  traffic cannot reset the deadline. These checks are not authentication.

### Supervision

Nonblocking Linux stdin/stdout replaces the blocking writer and background
reader/channel. Fixed-capacity framing bounds queued bytes/messages. Validated
`IoLimits` and `Timeouts` keep operational invariants private. A capped serializer
borrows the input Value, bounds depth/node visits and rejects oversized encoding
before any write. The Python SDK independently caps byte lines and output.

One monotonic deadline starts before serialization/launch preparation. I/O stops
at total minus cleanup reserve; cleanup consumes only the remaining reserved
time. No write/read transition, unrelated response or retry resets it. Equality
is expired. Bounded operations still depend on OS scheduling and syscall/loader
latency; this is not hard-real-time execution.

Timeout, malformed/oversized input, queue overflow, EOF and I/O errors retire the
session. Further sends return Unavailable. Application Remote errors and local
pre-write size/depth rejection leave the session usable. Healthy now means a
matching timely reply was observed and its next probe is not yet due. Ready
alone is Unknown, due confirmed sessions are Degraded, and expiry/failure is
latched Unavailable. Application Remote errors demonstrate responsiveness but
do not imply successful application behavior.
`WorkerSupervisor` charges every replacement attempt before launch, including
failed handshakes and launch failures, so the restart budget cannot be bypassed.

`HeartbeatPolicy` privately validates interval and response window (each
1ms..=60s; defaults 5s/1s). The window starts at the next due time, anchored to
the last host-observed confirmation; startup uses handshake completion only
as a scheduling anchor. Only trusted monotonic Instant is used. Checked time
arithmetic/regression faults retire the session. Equality at expiry rejects.
Periodic `WorkerSupervisor::poll` outside local control is mandatory, even when
idle. Late polling gets the remaining window; a missed window retires before
any new request/probe. No background timer or catch-up operation queue exists.
Ordinary request deadlines are capped by current health expiry; matching
response/error replies can substitute for probes. Both kinds share one sequence
and outstanding slot. Wrong kind/sequence, duplicate, stale or late traffic
cannot refresh health. The response-processing deadline and existing cleanup
reserve cover the entire operation. A later poll may make one replacement
attempt; it never hides a just-detected failure by retrying in the same call.
Fresh worker pipes/state start Unknown; old replies cannot restore a failed
session. Shutdown latches stopped and never implicitly relaunches.

Pre-issue draining shares that same absolute deadline and existing storage
bounds. Unsolicited current/future replies reject as protocol errors; stale
frames/logs cannot confirm responsiveness. Partial frames must complete before
a new write, so buffered preplayed data cannot become a fresh reply on issue.

Workers get their own process group. The library observes exit with WNOWAIT,
retaining the leader PID until group signalling, then closes stdio, sends group
SIGKILL and polls reaping without blocking wait. It requires exclusive ownership
of child reaping. Descendant-held pipes cannot retain I/O tasks: there are no
per-worker I/O threads. At most 32 live/deferred direct children share one bounded
reaper; unreaped children retain admission slots. CleanupReport records reaped,
deferred, ownership-lost or unavailable-reaper outcomes and signal errors.
Ordinary group descendants are signalled; escaped sessions and complete resource
containment remain out of scope. Repeated cleanup never signals a reused PID.

### Composition with FDIR

A worker's `HealthState` feeds `neuradix_safety::FdirMonitor`, so a Python crash
drives the system to a safe mode — the example shows crash → `Unavailable` →
FDIR `nominal → safe`, with the runtime surviving and restarting the worker.

### Python-side ergonomics

`python/neuradix_worker.py` provides `run(handler, name, skip_policy)`; a
component author writes an ordinary `handler(payload, config)` function. The
`skip_policy` declares whether input samples may be skipped (§19.4). Structured
config is delivered via `NEURADIX_WORKER_CONFIG`.

## Public interfaces affected

`neuradix-python`: `WorkerConfig`, `PythonWorker`, `ReadyInfo`,
`WorkerSupervisor`, `WorkerError`, `IoLimits`, `Timeouts`, `IoStats`,
`CleanupReport`, `HeartbeatPolicy`, `WorkerFailure`, periodic `poll` and
`check_heartbeat`, due/expiry accessors and `last_failure`. Python: `neuradix_worker.run`.

## Alternatives considered

- **In-process PyO3 embedding.** Rejected as the *first* step: embedding Python
  in the runtime process defeats crash isolation. PyO3/Maturin remains the plan
  for the zero-copy *binding* boundary (client types, NumPy views), which is
  additive to this process model.
- **Shared-memory / socket transport instead of stdio.** Deferred: stdio line-JSON
  is portable, simple and sufficient for control-plane messages; large-buffer
  paths will use the shared-memory data plane later.
- **Unbounded restart.** Rejected: a crash-looping worker must not restart
  forever; the budget latches failure (§16.8).

## Safety and security implications

`ResourceLimits` privately validates integral CPU seconds (1..=86,400, default
300) and virtual-address bytes (16 MiB..=1 TiB, default 256 MiB). AS rounds down
to system pages; native overflow/unlimited values reject explicitly. A required
absolute, operator-protected native helper installs equal soft/hard values and
reads them back. Supervisor limits never change. Linux procfs credential checks
reject root/capability-bearing execution; no_new_privs prevents exec privilege
gains. The child environment retains only PATH, LANG and explicit SDK settings.
Missing helper, unsupported platform or failed setup never launches unrestricted.
Setup failures carry a stage/errno. Observed exits carry a code or signal/core
flag without inferring CPU/memory exhaustion from generic crashes.

CPU time is lifetime process/thread CPU time, not elapsed time; ordinary forked
children inherit limits with separate CPU accounting. AS is virtual address
space, not RSS, process-tree or GPU memory. Workers cannot raise hard limits but
can lower them. The same policy is reinstalled on every budgeted replacement.
Only Linux GNU x86_64 is exercised in CI; other Linux configurations need separate
qualification. Aggregate/escaped-session containment is still incomplete.

Separate processes isolate Python exceptions/process exits from control and
safety; complete host resource isolation remains unimplemented. Python is kept
out of the deterministic executor (its
supervision uses wall-clock time and real processes, so it is non-deterministic).
The Linux bounded I/O path prevents worker-controlled pipe traffic from causing
an unbounded application wait or queue. It belongs outside the deterministic
control executor. Restart budgets include failed launch attempts. A05 tests
exercise continued local command gating and fallback while a worker hangs;
this is host software evidence, not physical rig validation. The resource suite
adds actual Linux per-process enforcement evidence within the scope above.

## Compatibility implications

The JSON protocol is versionable via the `kind` field and additive fields.
Adding PyO3 bindings is a new, separate surface, not a change to this one.
WP-A05 changes `send(Value)` to `send(&Value)`; makes WorkerConfig fields private
with builders; makes `with_request_timeout` fallible; adds validated limit/time
types; and returns cleanup reports from shutdown. Fatal failures now require
session replacement. Non-Linux launches return UnsupportedPlatform. The kind/
seq/payload envelope is unchanged; the SDK gains byte-limit environment settings.
Heartbeat users must update custom ping responders to the compact pong kind and
schedule periodic polling. The minimum 64-byte line limit supports a pong even
at the maximum u64 sequence. Health no longer treats process existence as proof;
ordinary requests may time out earlier at responsiveness expiry. Newly detected
failures return before a later recovery call; explicit shutdown prevents restart.
Resource users must build/install `neuradix-python-launcher` from the same package
and call `with_resource_launcher(absolute_path)?`. `with_resources` accepts only
validated `ResourceLimits`; getters expose requested and confirmed policy.
Ambient environment inheritance is restricted. The Python SDK is unchanged:
bootstrap is consumed before it executes. See the evidence document for commands.

## Testing strategy

`crates/python/tests/worker.rs` spawns a real `python3` and covers request
round-trip + config passthrough, **crash isolation and recovery**, supervisor
restart-budget exhaustion, and request timeout. New adversarial tests require
Python and run in externally timed subprocesses, covering blocked stdin,
oversized/flooded stdout, live-child EOF, descendants, total deadline accounting,
storage/admission bounds and failed launch budgets. CI also externally times
the Python/workspace commands and the migrated Python example. Exact evidence
and the limits of qualification live in the A05 implementation document.
Heartbeat tests add externally timed idle, stopped/hung, scheduling-gap,
duplicate/stale/late traffic, replacement, failed-launch, shutdown and independent
local-control scenarios. Deterministic boundary tests exercise the same private
monotonic state and deadline code at exact due/expiry and arithmetic limits.
Externally timed resource scenarios verify actual kernel limits, CPU termination,
AS allocation/mmap denial, inheritance, denied hard-limit raises, setup-before-code,
restart accounting and continued independent local control. A required privileged
negative test verifies helper rejection without ever acknowledging target exec.

## Unresolved questions

- PyO3/Maturin bindings and NumPy zero-copy views (§19.1–§19.2); wheel packaging.
- Content-addressed, locked Python dependency environments (§19.4).
- Aggregate process-tree CPU/RSS, GPU limits and escaped-session containment (§19.4).
- Additional OS/architecture qualification and deployed heartbeat scheduling.
- Graph-compiler detection of Python in a declared deterministic control path.
