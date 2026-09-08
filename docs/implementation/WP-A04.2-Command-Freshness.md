# WP-A04.2: command freshness and session validity

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

Verification is in progress on the branch; final CI identities and results will
be recorded here before requesting review. Focused tests cover shared host/embedded
boundaries, replay/exhaustion, renewal/restart, rejected-traffic liveness, idle
expiry, numeric invariants, arithmetic extremes, bounded state and serial binding.
The pinned CI workflow retains formatting, Clippy, workspace/doctests, docs,
independent no_std checks and the separate actual AVR codec conformance job.

## Remaining acceptance work

A04.3 must align host/MCU slew in physical units per time and test changing periods.
Rig-specific safe response, periodic executor/board integration, durable generation
allocation, stack/timing/resource measurements and physical reset/fault trials
remain unqualified. Compilation is not physical hardware validation. A04.1 and
A04.2 do not close WP-A04, ACC-05 or Gate A. No Studio, simulation backend,
enterprise subsystem or general authentication system is added.
