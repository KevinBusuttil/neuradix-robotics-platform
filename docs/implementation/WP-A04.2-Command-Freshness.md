# WP-A04.2: command freshness and session validity

Integration update: PR #9 merged as `c127c7d` after the review corrections below.
Corrected head `665fd5d` passed [CI 34288937294](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34288937294):
71 focused, 219 workspace tests/doctests and two AVR checks, with all other
workflow steps passing. [A04.3](WP-A04.3-Slew-Alignment.md) supersedes the retained
per-evaluation MCU slew semantics documented in this historical increment.


WP-A04 is **partial** and Gate A remains **open**. This increment adds bounded
command validity to the host and native embedded gates. Local control and fallback
have no AI, cloud, network service or allocation dependency on the embedded path.
Host/MCU slew alignment is reserved for A04.3.

## Reviewed integration baseline

[PR #8](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/8) was already
merged when this work started. Its reviewed head `5f7b7bd` and main merge
[`b4aae739`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/b4aae73979c620e3b7c674af1623a2b014eae612)
have identical source trees; [merge CI](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34283480925)
passed. No additional merge or review-rule bypass was performed. A04.2 starts
from that main revision on `codex/a04-2-command-freshness`.

The review retained A04.1's validated numeric constructors, final hard-bound
checks, runtime-owned evaluation time and latched evaluation-clock faults. One
concrete audit defect was found: JSON encoded a rejected NaN/infinity request as
`null`, which could not deserialize into the lineage's `f64`. Explicit string
markers now preserve that evidence, with round-trip regressions. The A04.1
[historical evidence and migration](WP-A04.1-Trusted-Evaluation.md) remain available.

## Defect and bounded design

Previously an authorized command could be replayed within its lease, and arriving
traffic could keep the embedded watchdog alive without a fully valid command.
Neither source age, source deadline nor session restart identity was enforced.
`neuradix-command-core` now owns the shared, allocation-free validation state;
both actual gates use it. Host tracking has at most 32 trusted holder/capability
bindings, including revoked tombstones. The embedded gate has one binding.
Payload identities only look up existing state; they never allocate entries.
Trusted host identity strings and ingress buffers still require deployment bounds.

The dependency-boundary test now includes `command-core -> time`, both gates
depending on `command-core`, and the existing serial crate's command binding
depending on `embedded-core/time`. No embedded crate gains a host-runtime or
std dependency; independent no_std CI checks enforce that configuration.

`CommandPolicy`, `SessionConfig`, `SharedTimeline` and `Generation` have private
invariant-bearing fields and validated constructors. No default policy silently
chooses a hazard budget: trusted configuration must supply positive maximum age
and watchdog timeout, nonnegative future skew, and a valid lease interval.

### Clock relationship and boundaries

`evaluate(input, now)` receives trusted runtime time (`TickContext.now` in the
host Processor). `CommandMeta.source_at` remains the original source timestamp.
Only an explicitly provisioned **shared reference clock and epoch** is supported:
the policy declares its nonzero timeline ID and domain, and source/deadline must
match both. Equal `Monotonic` or `Sensor` domain labels on independent devices are
insufficient. Independent device clocks and offset conversions are unsupported.
Trusted initialization must establish this relationship before enabling ingress;
an echoed timeline ID does not establish it. Receiver arrival time proves neither
source freshness nor clock synchronization. These checks are not authentication.

| Check | Exact acceptance boundary |
|---|---|
| Lease | `issued <= now < expires`; expiry equality rejects. |
| Maximum source age | `now - source_at <= max_age`; equality accepts. |
| Permitted future skew | `now - source_at >= -future_skew`; equality accepts. |
| Source deadline | `deadline > source_at` and `now < deadline`; expiry equality rejects. |
| Accepted-command watchdog | `now - last_accepted_at <= timeout`; equality accepts, next nanosecond rejects. |
| Sequence | First sequence may be any `u64`; subsequent values must strictly increase. Gaps are allowed. |

Source time is not an authority grant: lease authorization always uses `now`.
A source time before lease issue can pass only within the configured age window
and with the current generation. A deadline may exceed the lease or age window;
all checks apply, so the earliest limit wins. Timestamp subtraction uses checked
signed arithmetic, without absolute value or negation of `i128::MIN`.

### Trusted generations, renewal and restart

Each trusted holder/capability binding has one current generation. Commands echo
that generation, holder and capability; they cannot grant authority, renew,
replace, revoke or reset state. The host's string identities and embedded numeric
IDs must be bound by trusted deployment configuration. Payload claims and CRCs
are not proof of sender identity; any general authentication system is out of scope.

Before enabling ingress on **every** startup, trusted initialization must atomically
reserve and durably persist a nonzero `u128` generation greater than every value
previously used for that receiver/binding, then provision the gates and authorized
source with it. The same allocator must cover gate reconstruction, replacement,
revocation/regrant and recovery, including rollback or restored storage. Reserve
before activation so a crash cannot reuse an active value. Storage loss or uncertain
allocation requires remaining locally safe until a new, provably unused namespace
is established by trusted provisioning. No command can supply that decision.

The library enforces strictly increasing replacement within retained state; it
does **not** implement durable board storage or prove non-reuse across process
reconstruction. This is an explicit trusted integration requirement. A reset
counter, wall time without a durable guarantee, or value learned from traffic is
insufficient. Example generation `1` is for isolated deterministic demonstrations.
Generation `u128::MAX` cannot be replaced in the same namespace; wrapping is forbidden.

Same-generation renewal extends only expiry, while the existing lease is active.
Renewal is guarded by the gate's latest evaluation time and clock-fault state;
a delayed control-plane timestamp cannot revive observed expiry. The host entry
point is `SafetyGate::renew_lease`, not direct `LeaseTable` renewal. Core session
renewal requires the owning `EvaluationClock`; callers must retain that runtime
clock rather than constructing a new clock for each administrative request.
It preserves accepted sequence, source timestamp, deadline and watchdog time.
Expired or revoked leases require a greater generation. Revocation retains the
watermark and consumes its host slot; arbitrary eviction cannot reopen replay.
Replacement resets only that session's accepted-command state. It never clears
the gate's runtime-clock latch. After accepting sequence `u64::MAX`, the held
command remains valid until its time limits expire, but every new command receives
`SequenceExhausted`; a trusted greater generation is required to resume at zero.

### Validation and state updates

New-command checks run in this deterministic order:

1. Observe runtime clock. Domain change, regression and elapsed subtraction
   overflow latch `EvaluationClockMismatch`, `EvaluationTimeRegression` or
   `EvaluationTimeOverflow` until reconstruction with a non-reused generation.
   Equal evaluation time is allowed. Even rejected/idle ticks advance valid runtime time.
2. Match the trusted holder/capability (`UnknownBinding`); reject revoked,
   unsupported evaluation domain, not-yet-valid or expired authority.
3. Match generation, then shared timeline/source/deadline domains. Reject
   `GenerationMismatch` or `UnsupportedClockRelationship` explicitly.
4. Validate deadline ordering and expiry, checked source age, then future skew:
   `InvalidDeadline`, `DeadlineExpired`, `CommandTimeOverflow`, `StaleCommand`,
   `FutureCommand` respectively.
5. Compare last accepted sequence: exhausted first, then `Duplicate`, then `OutOfOrder`.
6. Reject `NonFiniteCommand`, check the optional host authority envelope
   (`OutOfEnvelope`), apply numeric constraints, and validate final finite hard
   bounds (`InvalidOutput`, including overflow).
7. Only complete acceptance commits sequence, source/deadline metadata and
   `last_accepted_at`, and applies the valid result.

Rejection immediately applies the validated local safe output, records runtime time
and a typed reason, and never refreshes accepted-command liveness. The fallback
remains active until a valid new command arrives; idle ticks do not resurrect the
previous nonzero output. Later idle expiry may replace the prior rejection reason.

On idle ticks, the gate checks its held binding/generation, lease, accepted source
age/deadline and watchdog, in that order. No accepted command yields `NoCommand`.
These checks do not consume sequence numbers or feed liveness. The host's
units-per-second and MCU's per-evaluation slew rules remain intentionally separate
pending A04.3; shared conformance scenarios isolate validity from those rules.

### Required runtime scheduling

Call the gate on a bounded periodic schedule, including `evaluate(None, now)` when
no command arrives or a frame fails decoding. Apply each returned output locally.
Do not wait for traffic, feed an actuator watchdog from raw arrival/CRC success,
or let input floods starve evaluation ticks. Process a bounded amount of ingress
per tick. Configure period and jitter against the rig's safe-response budget:
expiry is observed at the first evaluation at/past its boundary, so scheduling
can add up to one period plus jitter. Trusted grant/revoke/renew operations must
be followed by evaluation before actuation. The existing input-driven lockstep
executor has no background timer; its caller must supply idle ticks. Physical
watchdog clocks must keep advancing if simulation or host execution pauses.

## API and serial compatibility

| A04.1 interface | A04.2 migration |
|---|---|
| Public lease fields / `until` / priority-based same-binding grants | Construct a validated `SessionConfig`, then `AuthorityLease::new`. One current session per binding; arbitration/priority must be resolved by trusted control-plane policy before provisioning. |
| Infallible `LeaseTable::grant` / direct `authorize` | `grant` returns `Result`, enforcing capacity and greater generation; `renew` extends active authority without resetting state. Gate evaluation performs authorization plus command validation. |
| `CommandRequest.at` | `CommandRequest.meta` contains generation, sequence, original `source_at`, exclusive deadline and timeline. Constructor's final argument is now `CommandMeta`. |
| `SafetyGate::evaluate(request, now)` / Processor input | Both accept `Option<CommandRequest>`; `None` is an idle expiry tick. `SafetyDecision.request` is optional. |
| Embedded `Option<f32>` / `CommandGate::new(..., watchdog, safe)` | `Option<Command>` carries holder/capability and metadata; constructor takes `(limits, lease, safe)`. Timeout lives in `CommandPolicy`. Standalone `Watchdog` remains a separate utility. |
| Gate-specific rejection variants | Both gates use shared `CommandRejection` (host `RejectReason`, MCU `SafeReason` aliases). See rustdoc for all reasons. |
| `CommandLineage::from_decision -> Self` | Returns `Option<Self>` (idle decisions have no invented source). Record idle `SafetyDecision`/`GateDecision` reasons separately if required by deployment. |
| Lineage JSON | Adds optional `commandMetadata` containing original source/deadline clocks, generation as decimal text, timeline and sequence. Older records without it still read. Non-finite `requested` uses `"NaN"`, `"Infinity"`, `"-Infinity"`; finite values remain numbers. |

The existing serial frame's CRC/header layout is unchanged. The new version-1
command payload is exactly **87 bytes**, encoded little endian by `encode_command`
and decoded by `decode_command`: version(1), holder(8), capability(8), generation(16),
sequence(8), timeline(8), source domain(1)/nanoseconds(16), deadline
domain(1)/nanoseconds(16), value(4). Both endpoints must migrate; legacy four-byte
scalar commands are rejected by this binding. Unknown versions/domains, wrong size
and zero generation fail decoding. Invalid finite/value or temporal semantics
reach the gate for a typed decision. Outer `u16` frame sequence is diagnostic;
only the inner `u64` sequence controls replay rejection. Original source time is
preserved across the link. No new network transport or scalar codec is introduced.

## Verification evidence

Verified implementation commit:
[`41ad541b`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/41ad541b253412d74e0a63239b8d07a744b5eb21).
[PR CI run 34287037280](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34287037280)
and [branch CI run 34287035067](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34287035067)
passed. [PR #9](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/9)
tracks this unmerged increment; subsequent evidence-only edits do not change the
tested implementation.

| Check | Observed result |
|---|---|
| Pinned tools | Rust 1.94.1 (`e408947bf`), Cargo 1.94.1 (`29ea6fb6a`), locked dependencies; CI `RUSTFLAGS="-D warnings"`. Required host g++ 13.3.0 and Python 3.12.3 were present and their conformance tests executed. |
| Format and lint | `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings` passed. Dependency-boundary test includes the shared crate while forbidding host-runtime dependencies from embedded crates. |
| Focused tests | `cargo test --locked -p neuradix-command-core -p neuradix-safety -p neuradix-embedded-core -p neuradix-embedded-transport`: **69 passed, 0 failed, 0 ignored**. This is a workspace subset, not 69 additional unique tests. |
| Workspace | `cargo test --locked --workspace`: **216 passed, 0 failed, 2 ignored**. The two AVR tests execute explicitly in the separate job below. |
| Migrated examples | `cargo run --locked -p` each of `neuradix-example-minimal-depth-stream`, `neuradix-example-auv-depth-sim`, `neuradix-example-embedded-propulsion` passed. Deterministic host execution; no hardware claim. |
| Independent no_std configurations | Separate `cargo check --locked -p <crate> --no-default-features` passed for `neuradix-time`, `neuradix-command-core`, `neuradix-embedded-core`, `neuradix-embedded-transport`, outside workspace feature unification. Host-target no_std compilation, not native MCU execution. |
| Documentation | `cargo doc --locked --workspace --no-deps` passed. Local changed-document check passed **312 relative paths/anchors across 8 files**, and `git diff main --check` passed. |
| AVR conformance | `cargo test --locked -p neuradix-embedded-codegen --test avr -- --ignored --nocapture`: **2 passed**, AVR GCC 7.3.0. Actual ATmega328P scalar harness compile/link and expected binary64 ABI rejection. Unchanged codec footprint: 2,778 text + 322 data = 3,100 flash bytes; 6 bss + 322 data = 328 static SRAM bytes. This is not an A04.2 gate footprint or a board trial. |
| Earlier failed checks, fixed | [First run](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34286439287): rustfmt differences. [Second run](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34286565346): missing lineage metadata in CLI fixtures. [Third run](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34286776149): dependency map omitted the new shared crate/binding. All were corrected; the verified implementation passes CI. |
| Pre-existing failed documentation scan | A broader scan found **32 broken archived links**: 31 missing image references in Specifications v0.4/v0.5, plus the v0.1 plan's absent Specification v0.2. Those historical files are unchanged by this PR; they are not counted as a passed whole-repository link check. |
| Unavailable / not run | Local Cargo/rustup and AVR tools were absent, and local Rust installation network access was unavailable; executable checks ran in repository CI. No physical Arduino/MCU, HIL rig, durable-storage driver, physical watchdog timing, stack measurement or reset trial was tested. |

The 15 paired regressions in `crates/safety/tests/freshness.rs` drive both actual
gates through common validity scenarios: exact age/skew/deadline/lease boundaries,
valid progression and gaps, duplicates/order/exhaustion, renewal/restart and old
generations, unknown identities/capabilities, explicit clock relationships,
latched clock faults, arithmetic extremes, rejected-traffic liveness and idle
expiry. Core configuration tests, migrated A04.1 numeric suites, lineage JSON
round trips and serial framing-to-gate tests cover the remaining boundaries.

## PR review corrections

Review of head `48dde8ba` identified two defects corrected before integration:
- Stale control-plane renewal time could revive an expired generation. Both gate
  renewal paths now check the owning evaluation clock; stale, incompatible or
  faulted clock state returns `InvalidEvaluationTime` without changing the lease.
  Host renewal moved from `LeaseTable` to `SafetyGate::renew_lease`.
- CLI `explain command` reserialized non-finite requested values as null, and
  numeric series could aggregate them. Explain now retains the explicit markers;
  a series selecting a non-finite field returns a decode error with field/time.
  Valid applied-output series remain available. No Studio feature was added.

Paired gate regressions cover expiry followed by delayed renewal and latched
faults; an end-to-end CLI test covers all three non-finite markers and series.
The earlier CI results above describe the pre-review revision; corrected-head
checks are recorded in PR #9 before merge.

## Remaining acceptance work

A04.3 must align host/MCU slew in physical units per time and test changing periods.
Rig-specific safe response, periodic executor/board integration, durable generation
allocation, stack/timing/resource measurements and physical reset/fault trials
remain unqualified. Compilation is not physical hardware validation. A04.1 and
A04.2 do not close WP-A04, ACC-05 or Gate A. No Studio, simulation backend,
enterprise subsystem or general authentication system is added.
