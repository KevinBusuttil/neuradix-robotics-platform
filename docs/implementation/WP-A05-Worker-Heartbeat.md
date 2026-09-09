# WP-A05: heartbeat health and bounded recovery

This is a bounded increment of **WP-A05: Bounded extension processes**.
WP-A05/ACC-09 remain partial. Comprehensive CPU/memory/GPU enforcement,
escaped-session containment, additional OS backends and deployment supervision
remain open. WP-A04 is partial, ACC-05 incomplete and Gate A open; no physical
response, board timing, durable-startup or resource qualification is added here.

## Reviewed prerequisite and baseline

[PR #11](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/11) was
open at the previously observed `589dcaaa` head. Review found a concrete SDK
defect: a 128-character error diagnostic could exceed a valid 64-byte envelope
and kill an otherwise recoverable worker. Fix `063d0d77` bounds the complete
encoded diagnostic, including sequence width, escaping, UTF-8 and newline.
All five SDK tests passed locally; [current PR CI 34318590924](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34318590924)
passed every host and AVR check. The duplicate push host job also passed; its
AVR job was queued at integration review. Repository rulesets were empty, main
was unprotected and no external approval was required. PR #11 merged as
`e12420aed4886c35f41c3034646282e4b1dba49a`; its tree equals the reviewed fix tree.
Its [merge CI 34318755055](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34318755055)
also passed.

The isolated `codex/a05-worker-heartbeat` branch starts from that updated main.
Existing branches/checkouts were preserved; no applicable AGENTS.md was found.
WP-A01's integrated baseline, architecture checks and pinned CI support this
work; its broader evidence inventory/optional-tool audit remain partial.
The [bounded I/O evidence](WP-A05-Bounded-Worker-IO.md) describes the retained
storage, process admission, cleanup and OS assumptions.

## Defect and policy

The worker previously answered an SDK ping but the supervisor never scheduled
one. `Healthy` meant the process existed: an idle, stopped or hung process could
remain apparently healthy indefinitely. Request timeouts only detected failure
when an ordinary request arrived. Heartbeat scheduling is now explicit and
supervisor-owned; worker timestamps and arbitrary stdout do not supply time or
responsiveness evidence.

| Configuration | Default | Validated meaning |
|---|---|---|
| `HeartbeatPolicy::interval()` | 5 seconds | 1ms..=60s after last confirmation, or initial ready handshake |
| `HeartbeatPolicy::response()` | 1 second | 1ms..=60s response-processing window after due time; equality expires |
| Cleanup reserve | Existing `Timeouts::cleanup()` (50ms default) | Additional to heartbeat response window; remains part of ordinary request totals |
| Outstanding operations | Exactly one | Requests and pings share exclusive `&mut self`, sequence and bounded stdio; no extra queue/thread |

Both policy fields are private. A validated policy can only be installed at
worker construction; commands cannot alter it. All time is trusted host
`std::time::Instant`, unrelated to A04 source/evaluation timestamps or replay
clocks. Checked arithmetic and regressing observations fail closed with a
latched `Clock` reason. Durations below 1ms or above 60s are rejected.

For most recent confirmation time **C**, interval **I**, and response budget
**B**, the next probe is due at **D = C + I**, and responsiveness expires at
**E = D + B**. Initial scheduling uses ready-handshake completion as C but marks
the state **Unknown**, not Healthy. Only a matching timely response establishes
positive health. Confirmation uses the host observation time after complete
bounded parsing and deadline validation. Equality at E rejects, including a
reply already in a kernel or user-space buffer but not processed in time.
Final deadline and confirmation checks share one captured observation so a
boundary crossing between two clock reads cannot change the failure category.

### Scheduling and one absolute deadline

Call `WorkerSupervisor::poll()` at or before `heartbeat_due()` even when there
are no ordinary inputs. Before D it does no pipe I/O. At/after D and before E,
it performs one ping with I/O deadline E and cleanup end E + reserve. A late
poll gets only E − now; it never starts a fresh B budget. At/after E, it issues
no ping, reports `HeartbeatExpired` and retires the session within the cleanup
reserve from observation. Repeated queries cannot revive it. No background
timer exists: if the embedding service stops calling, physical action cannot
occur at E; the next observation detects the missed window. A deployment must
schedule polling and consume health outside conventional local-control execution.

For an ordinary request starting S with total T and reserve R, I/O and complete
response processing stop at **min(S + T − R, E)**; cleanup ends by
**min(S + T, E + R)**. The same absolute deadline covers bounded serialization,
partial writes, unrelated responses and cleanup. A long handler cannot occupy
the single outstanding slot beyond its current responsiveness window. A timely
ordinary response or matching application error substitutes for a heartbeat
and schedules a new D/E. Local pre-write size/depth rejection does not consume
a sequence or refresh health. Requests in the due interval may still succeed
before E; expired requests cannot send payloads to resurrect a session.

Before issue, the same deadline also bounds draining queued/kernel traffic.
Complete and partial frames must finish before a new write. A reply naming an
as-yet unissued sequence is a terminal protocol error; old sequence/log traffic
is discarded without confirmation. This prevents a prequeued or partial
preplayed reply from being counted after issue. A stream that never drains can
exhaust this deadline, but cannot extend it or allocate beyond existing limits.

Bounds cover application loops and work quanta, not preemption of scheduler
suspension, allocation, bounded parsing, process creation or kernel calls.
They are ordinary Linux software bounds, not hard-real-time guarantees. A
supervision call detecting failure returns it before replacement; a later call
may perform one bounded launch. There is no combined retry loop or catch-up
burst for missed heartbeat intervals.

### Reply validation and health

Validation order is: available session and monotonic health window; direct-child
status; next checked sequence; bounded serialization; bounded pre-issue drain;
bounded write/read under
the same deadline; complete JSON object; exact sequence and operation-specific
kind; final deadline/window check; then confirmation. Wrong sequence/kind does
not refresh health or extend a deadline. Size/queue/parser faults still retire
the session even when the same burst contains a valid reply.

| Input or event | Health/state effect |
|---|---|
| `ready` declaration | Unknown; initializes a scheduling window only |
| Matching `response` to ordinary request | Confirms responsiveness, returns payload |
| Matching `error` to ordinary request | Confirms responsiveness, returns `Remote`; application correctness is separate |
| Matching `pong` to outstanding `ping` | Confirms responsiveness |
| Ordinary `response`/`error` during ping, pong during request, wrong sequence, duplicate/stale reply, logs | No confirmation; same deadline remains |
| Confirmed, now < D | Healthy |
| Confirmed, D <= now < E | Degraded: probe due; unconfirmed startup remains Unknown |
| Ping I/O reaches E | `HeartbeatTimeout`; terminal Unavailable |
| Call resumes at/after E | `HeartbeatExpired`; terminal Unavailable |
| Ordinary request deadline | `Timeout` / `RequestTimeout` reason; terminal Unavailable |
| Exit, EOF, queue/size/protocol/I/O fault or sequence exhaustion | Explicit error and first latched terminal reason |
| Explicit shutdown/Drop | Unavailable; preserves an earlier failure, otherwise records Shutdown |

Sequence starts at 1, increments for both request kinds when a write is
attempted, and never wraps; exhaustion retires the session for replacement.
Only one sequence is outstanding. A worker can lie about its own behavior:
matching protocol replies demonstrate observed responsiveness, not successful
application work, cryptographic identity or authentication. This is not a
security sandbox. Stale replies do not match a later sequence. A replacement
gets new pipes, framing, sequence and health state; no buffered reply or
confirmation from the old process is transferred. Ordinary descendants holding
old descriptors cannot access the new supervisor pipe by retaining the old one.

### Recovery and ownership

`WorkerFailure` records the first terminal category for a worker;
`last_failure()` survives health queries, late traffic and repeated shutdown.
The supervisor retains its most recent failed-session/launch reason even after
a successful replacement; that audit value is separate from current health.
Every replacement attempt consumes the existing restart budget before launch,
including admission, executable and handshake failures. Exhaustion does not
reset on healthy replies or successful launches. `poll` detects first, then a
later `poll` or `ensure_alive` attempts one replacement. A ready replacement is
Unknown until a matching timely reply. Explicit supervisor shutdown latches
stopped; neither polling nor ensure_alive restarts it.

All methods require exclusive mutable ownership. Shutdown/Drop cannot race an
outstanding heartbeat through the safe API; the heartbeat first completes or
fails within its budget. Failure performs terminal cleanup, so subsequent
shutdown/Drop reuses the cleanup report without another signal/wait. Direct
Drop of a live worker remains bounded by the existing cleanup reserve.

No additional background work or process slots were added. Existing nonblocking
stdio, line/queue limits, 32 live/deferred admissions, single bounded reaper,
exclusive child-reaping ownership and process-group cleanup are unchanged.
Linux GNU/musl remains the implemented backend; only Linux GNU CI execution is
qualification here. Other OSes reject launch. Escaped sessions and comprehensive
CPU/memory/GPU containment remain unimplemented.

## API and protocol migration

- Add `.with_heartbeat(HeartbeatPolicy::new(interval, response)?)` to customize
  defaults; `WorkerConfig::heartbeat()` reads the immutable policy.
- Schedule `WorkerSupervisor::poll()` or standalone
  `PythonWorker::check_heartbeat()` during idle periods. `heartbeat_due()` and
  `heartbeat_expires()` expose trusted host instants for scheduling/diagnosis.
  No worker-supplied timestamp API is introduced.
- `health()` reports responsiveness, not process existence. It can retire an
  expired session using the reserve, so keep it outside local control too.
- `is_running()` means an available, unexpired session; it is not a heartbeat.
  `ensure_alive()` does not issue pings and now returns newly observed failures
  before attempting recovery on a later call. Shutdown prevents automatic restart.
- `last_failure()` and `WorkerFailure` expose bounded audit categories. Detailed
  `WorkerError` and existing `CleanupReport` retain operation/cleanup details.
- The Python SDK now replies to `{"kind":"ping","seq":N}` with
  `{"kind":"pong","seq":N}`. The previous response/payload/pong shape does
  **not** satisfy a probe. Update custom workers with the SDK. The compact pong
  fits the minimum 64-byte limit even at u64 maximum; request/response/error and
  shutdown envelopes are otherwise unchanged.
- No new dependency, transport, command authority, embedded or no_std API change.

The Python example shows initial Unknown, ordinary confirmation, idle polling,
failure-to-FDIR and bounded recovery on an independent supervision path. FDIR's
conservative treatment of Unknown/Unavailable is unchanged; deployments must
choose startup handling and scheduling explicitly, without suppressing faults.

## Verification evidence

Implementation `58e2b224` (including the preplayed-frame correction) passed
[PR CI 34320430371](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34320430371)
and [push CI 34320426971](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34320426971).
[PR #12](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/12) records
current-revision checks after the final shared-observation/API documentation
correction. It subsequently merged as `5cda65a`, with unchanged head `0cd31b7`
and passing current PR/push CI `34320795164`/`34320791968`. The subsequent
[resource increment](WP-A05-Worker-Resource-Limits.md) records that integration
review and adds per-process Linux CPU/AS policy. CI used pinned Rust/Cargo **1.94.1**,
locked dependencies, warnings denied, G++ 13.3.0, Python 3.12.3 and AVR GCC 7.3.0.

| Check | Result |
|---|---|
| `cargo fmt --all --check` | Passed |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Passed |
| Command-core/safety/embedded-core/embedded-transport focused regressions (`cargo test --locked -p ...`) | **88 passed**; A04.1/A04.2/A04.3 regressions preserved |
| `timeout --signal=TERM --kill-after=5s 120s cargo test --locked -p neuradix-python` | **55 top-level tests/doctests passed**, including 16 heartbeat and 20 bounded-I/O subprocess scenarios |
| `timeout --signal=TERM --kill-after=5s 30s python3 -B -m unittest discover -s python/tests -v` | **6 SDK tests passed**, also passed independently locally |
| `timeout --signal=TERM --kill-after=5s 180s cargo test --locked --workspace` | **287 top-level tests/doctests passed**; two AVR tests ignored here and executed separately |
| Four `cargo run --locked -p ...` examples: minimal-depth-stream, auv-depth-sim, embedded-propulsion, python-worker | All passed; Python example externally bounded to 30 seconds |
| Independent `cargo check --locked -p ... --no-default-features`: time, command-core, embedded-transport, embedded-core | All four passed |
| `cargo doc --locked --workspace --no-deps` | Passed |
| `cargo test --locked -p neuradix-embedded-codegen --test avr -- --ignored --nocapture` | **2 passed**, ATmega328P scalar compile/link and intended binary64 rejection |
| Changed Markdown paths/anchors and `git diff --check` | Passed: seven files, 319 local paths/anchors, no missing targets |

Focused suites overlap workspace. The 36 nested scenario-child reports execute
their parent tests and are not counted twice; SDK and separate AVR passes are
additional. No dependency versions or embedded execution paths changed.

Earlier [CI 34319531631](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34319531631)
and [CI 34320302532](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34320302532)
failed formatting; pinned formatter output was applied. The initial heartbeat
implementation passed both runs before final review added the concrete
preplayed-reply correction and additional regressions; the named later runs
include that correction. No failed checks are represented as passes.

The full archive Markdown scan still fails: **46 files, 627 local references,
32 pre-existing missing targets** (31 figure links in specifications v0.4/v0.5
and one v0.2 specification link in plan v0.1). These unrelated files were left
unchanged. Rust/rustup/Cargo and AVR tools were unavailable locally; repository
CI supplied compilation/test evidence. Physical rigs, other OS/architecture
execution and musl qualification were unavailable. Host tests and AVR compilation
are not physical hardware validation, measured worst-case response, or complete
resource containment evidence.

New real-worker scenarios have a 10-second external subprocess timeout and a
200ms CI scheduling allowance. Configured interval/response/reserve are usually
60/250/50ms; late-poll testing uses 700ms with 500ms consumed before calling,
so resetting the response window fails the test. Exact nanosecond due/expiry,
monotonic regression and representable-Instant arithmetic boundaries use the
same private state/deadline code with deterministic supplied observations.
Real worker tests exercise actual public gates; injected test time is not a
public worker time source. Existing queue/admission/reaper/descendant regressions
are rerun, with another actual local SafetyGate/fallback test during heartbeat
failure. The SDK suite independently checks compact pongs and handler isolation.

## Remaining acceptance

Resource-limit policy/enforcement and deployment/platform qualification remain
WP-A05/B07 work. ACC-09 requires explicit resource outcomes and complete claimed
platform evidence; heartbeat and I/O results alone do not close it. A04 physical
rig response, board timing/restart behavior, durable startup and stack/resource
measurements remain separate. WP-A05 and WP-A04 stay partial; Gate A stays open.
