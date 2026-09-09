# WP-A05: bounded Python worker I/O and cleanup

This is one bounded increment of **WP-A05: Bounded extension processes**.
WP-A05 and ACC-09 remain partial: heartbeat policy, comprehensive resource
enforcement, other OS backends and deployment supervision are deferred.
WP-A04 remains partial, ACC-05 incomplete and Gate A open while physical response,
board restart/timing, durable startup and resource evidence remain unavailable.

## Reviewed baseline

[PR #10](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/10) was
reviewed at unchanged head `a81fab7e3339e56eece3bce7349d357563423499`.
Its current [PR CI 34290766316](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34290766316)
and push CI passed; there were no review comments or blocking repository rules.
The shared slew arithmetic, actual-gate conformance, migration and remaining
physical acceptance matched the implementation. No prerequisite code defect
was found. PR #10 merged as `9dafdb7e3fd692c229bf12db5a0f39202bcf957e`.

The isolated `codex/a05-bounded-worker-io` checkout starts from that updated main;
existing checkouts and branches were preserved. No applicable AGENTS.md was
found. WP-A01 supplies the integrated baseline and architecture/CI checks; its
broader capability inventory and optional-tool audit are still partial.

## Concrete defects and design

The old worker wrote and flushed stdin before starting its response deadline.
It used unbounded `read_line` and `mpsc::channel` storage, and called blocking
`Child::wait` after stdout closed, even if the child remained alive. Shutdown
could block while writing its shutdown message. Failed restart launches were
not charged against the restart budget.

The Linux implementation now uses nonblocking existing stdio pipes and bounded
framing, without per-worker reader/writer threads. A 4 KiB write quantum and
1 KiB read quantum allow both directions to progress; each loop checks the
same monotonic `Instant` deadline. There is no new transport or wire envelope.
The library continues to forbid unsafe code; safe `nix` 0.30.1 wrappers provide
fcntl, waitid and process-group signalling. The lockfile adds nix, bitflags and
cfg_aliases without upgrading existing dependencies.

### Bounds and validation

| Configuration | Default | Valid range / meaning |
|---|---|---|
| Incoming line | 65,536 bytes | 64..=1,048,576, including newline |
| Outgoing line | 65,536 bytes | 64..=1,048,576, including envelope and newline |
| Receive byte storage | 262,144 bytes | At least one maximum incoming line, at most 4,194,304; includes complete and partial lines |
| Complete queued lines | 16 | 1..=128; one partial line also shares the same byte budget |
| Outstanding requests | 1 | Exclusive synchronous `&mut self`; no outbound request queue |
| Handshake / request total | 10 seconds each | Greater than cleanup reserve, at most 60 seconds |
| Shutdown total | 500ms | Greater than cleanup reserve, at most 60 seconds |
| Cleanup reserve / Drop total | 50ms | Nonzero, at most one second |
| Live + deferred direct children | 32 per process | One shared admission pool; full/busy admission returns `ProcessCapacity` before spawning |

`IoLimits` and `Timeouts` are immutable validated types. `WorkerConfig` fields are
private with builders; payloads cannot mutate configuration. A line at the byte
limit must end in newline; an unterminated line rejects before further growth.
Queue saturation returns an explicit byte/message overload error. A receive
burst is framed before lines from that burst are consumed; even a burst containing
a valid response may overload. There is no silent dropping to hide overload.

Fixed-capacity receive storage is bounded by configured queued bytes plus line
length metadata. Processing may additionally hold one incoming line and its
decoded JSON; one bounded outgoing line and bounded handshake metadata are
separate. Allocator/JSON object overhead is not an exact heap-byte quota.
Kernel pipes are separately bounded by the OS. Arbitrary worker memory/CPU/file
creation and the caller's already-created `Value` are not contained here.

Serialization borrows the caller's Value, limits recursion to depth 32 and node
visits to the outgoing-byte budget, and uses a capped writer before any pipe
write. Incoming serde JSON retains its recursion guard. The companion Python
SDK uses bounded byte-line reads and capped UTF-8 output, with finite JSON and
explicit malformed/startup-config failure; a hostile worker cannot bypass the
independent Rust-side checks by ignoring SDK environment limits.

### Deadline and state policy

For start time `S`, configured total `T` and reserve `R`, I/O and response
processing stop at `S + T - R`; cleanup stops by `min(now + R, S + T)`.
Equality is expired. Serialization, partial writes, unrelated lines and cleanup
never start a fresh request budget. Shutdown has its own declared total;
Drop uses only the reserve. Repeated cleanup returns its original report without
another wait or another signal to a potentially reused PID.

Deadlines bound application loops, storage and work quanta. They do not preempt
OS spawn/loader/kernel calls, allocation, a bounded JSON operation or scheduler
suspension. This is an ordinary Linux host guarantee, not hard real time.
Call the worker API outside the conventional local-control executor.

| Event | Outcome / state update |
|---|---|
| Valid handshake | Require an object with `kind=ready` and string name/skipPolicy |
| Valid matching response | Return payload; only one request is outstanding |
| Matching application error | `Remote`; worker remains usable |
| Outbound size/depth/node rejection before write | Explicit local error; no bytes written and no sequence consumed |
| Handshake timeout | `HandshakeTimeout`, terminal cleanup |
| Request timeout, malformed/oversized input, queue overflow, I/O failure | Explicit reason, terminal cleanup; further sends return `Unavailable` |
| Live child closes stdout | `StdoutClosed`; never wait for it to exit voluntarily |
| Observed child exit | `WorkerExited`; EOF during exit can precede observed exit and report `StdoutClosed` |
| Wrong sequence or unrelated kind | Ignore within the same deadline; no liveness claim |
| Sequence exhaustion | Reject `SequenceExhausted`; never wrap; explicitly shut down/restart the session |
| Failed replacement launch/handshake | Consume the restart attempt before launch; no unlimited retry path |

Sequences begin at 1 and are consumed when a write is attempted. A terminal
failure prevents reuse of a partly written session. Sequence checks identify
responses; they are not authentication and confer no command authority.
`Healthy` still means an available running process, not heartbeat/handler health.

### Cleanup and OS scope

Linux GNU/musl uses a new process group for each worker. Status observation uses
`waitid(WNOWAIT|WNOHANG)`, keeping the leader's PID reserved until group cleanup.
The supervisor closes its stdin/stdout handles, signals SIGKILL to the group
and polls direct-child reaping without a blocking wait. Descendant-held pipes
cannot retain supervisor I/O tasks, because those tasks do not exist.

If reaping does not finish within the reserve, one process-wide reaper thread
polls the child in bounded storage. Its admission token is retained until actual
reaping; stuck children can exhaust 32 slots but cannot create unbounded threads,
queues or additional admitted workers. `CleanupReport` distinguishes Reaped,
Deferred, OwnershipLost and ReaperUnavailable, plus any signal error. Non-normal
operation cleanup preserves the original failure in `WorkerError::Cleanup`.

The embedding process must leave child reaping to this library: no external
`waitpid(-1)`, SIGCHLD auto-reaping or competing reaper. Observed ownership loss
avoids signalling a potentially reused PID. Process-group signalling covers
ordinary descendants retaining that group; deliberately escaped sessions and
comprehensive containment are deferred. A group signal is not proof every
descendant has exited. Same-user process isolation is not a security sandbox.

Other OSes return `UnsupportedPlatform` before launching. No alternative blocking
fallback or unsupported containment guarantee is advertised. Only Linux CI
execution is evidence here; musl/other architecture qualification remains open.

## API migration

- `send(Value)` becomes `send(&Value)` to avoid owning/cloning arbitrary payload
  trees. The JSON line envelope remains kind/seq/payload.
- Direct WorkerConfig field writes become builders. `with_request_timeout`
  now returns Result; `with_timeouts(Timeouts::new(...)?)` sets total budgets.
- `with_limits(IoLimits::new(...)?)` sets validated wire/storage budgets.
- `PythonWorker::shutdown` returns CleanupReport; supervisor shutdown returns
  Option<CleanupReport>. Inspect deferred or failed cleanup explicitly.
- `io_stats()` exposes observed queue/outgoing high-water marks;
  `cleanup_report()` retains cleanup diagnosis after terminal failure.
- Timeout/protocol failures now retire the worker; recover through the existing
  bounded supervisor. No change to A04 command-gate semantics or embedded APIs.

## Verification evidence

Pinned CI and final evidence are in progress in [PR #11](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/11).
Local Rust/rustup/Cargo and AVR tools are unavailable; repository CI supplies
compiler/test evidence. Local Python syntax parsing and whitespace checks run
independently. Exact passed, failed and unavailable checks will be recorded
before this PR is marked ready for review.

Adversarial cases execute in separate test subprocesses with 10-second external
timeouts. Fixture workers self-expire to clean up failed runs. CI also applies
external 120-second Python and 180-second workspace test limits. Assertions use
300ms operations plus a declared 400ms CI scheduling allowance; the combined
write/read deadline test uses 1 second plus 250ms. These are test tolerances,
not physical timing qualification or measured worst-case execution times.

## Remaining acceptance

General heartbeat/idle responsiveness policy, comprehensive CPU/memory/GPU
enforcement, escaped-session containment, additional OS backends and deployment
supervision remain WP-A05/B07 work. ACC-09 cannot close on I/O tests alone.
Physical control response, board timer/reset behavior and resource qualification
remain separate A04/ACC-05 evidence. Gate A stays open.
