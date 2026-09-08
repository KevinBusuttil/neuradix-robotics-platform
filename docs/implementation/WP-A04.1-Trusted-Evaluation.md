# WP-A04.1: trusted command evaluation and validated numeric configuration

This increment implements the trusted-time and numeric-configuration portion of
[WP-A04](../Neuradix_Implementation_Plan_v0.4.md#wp-a04), contributing to
NRX-PLAT-006 / ACC-05. **WP-A04 is partial and Gate A remains open.** Conventional
control and fallback remain local, independent of AI, cloud and UI availability.

## Baseline and defect

The branch starts from main `e95a0ac9abaee279ff59296302344f01dc3bcec1` (including
the integrated code at `d5fbd69`). No applicable `AGENTS.md` was present in the
repository or checkout ancestors. Existing checkouts were left untouched.

On this baseline, host `SafetyGate::evaluate(request)` authorized the lease,
computed elapsed time and timestamped decisions using sender-controlled
`request.at`; `Processor` ignored `TickContext.now`. A command stamped inside an
old lease could therefore be accepted after its expiry at actual evaluation.
Host range/envelope/rate configuration allowed non-finite values or unchecked
public construction. Embedded `Limits::new` already validated numbers, but its
public fields bypassed validation; `CommandGate::new` silently clamped or
replaced an invalid safe output. These were executable defects, not only missing
documentation. No A04.1 fix was present on the baseline.

## Resulting time policy

| Input or event | Defined behavior |
|---|---|
| Runtime evaluation | Host `evaluate(request, now)` uses `now` for lease authorization, elapsed time and `SafetyDecision.at`; `Processor` passes `TickContext.now`. Embedded evaluation retains its explicit runtime `now`. |
| Source timestamp | `CommandRequest.at` remains unchanged in `SafetyDecision.request`. It has no authority or elapsed-time effect. A different source clock domain or source regression is diagnostic metadata, not an evaluation clock fault. Source age/future-skew validation is deferred. |
| Lease clock | Both host lease endpoints must match the evaluation domain. If no matching lease is comparable, reject with `Authority(ClockDomainMismatch)`. A valid comparable lease can still win when unrelated-domain leases exist. Embedded cross-domain lease checks retain `LeaseExpired`. |
| Evaluation domain change or regression | From the first evaluation, each gate tracks its runtime clock. A domain change or decreasing time latches `EvaluationClockMismatch` / `EvaluationTimeRegression`, applies safe output, and cannot be cleared by later apparently valid time. Host reasons use the `RejectReason` names; embedded uses `SafeReason`. |
| Elapsed-time overflow | Host checked timestamp subtraction failure latches `EvaluationTimeOverflow`; no zero-duration substitution or absolute-value conversion. Embedded watchdog arithmetic remains saturating and slew remains per evaluation. |
| Equal evaluation times | Allowed. Host elapsed time is zero, so slew holds the previous output. Embedded retains its existing per-evaluation step semantics. |
| Clock fault recovery | Construct a new gate through trusted runtime lifecycle/configuration with appropriate leases and clock state. This increment provides no reset/epoch protocol. Command data must never trigger reconstruction. |
| Ordinary rejection | Apply validated safe output immediately, bypassing slew. Host records the valid evaluation time so recovery slews from the safe output at that time. A clock fault keeps the last valid reference and remains latched. |

The first evaluation, including an authority rejection, establishes the runtime
clock reference. Lease expiry is exclusive. A regression after observing expiry
cannot resurrect the lease. Decisions on clock faults record the supplied runtime
time (even though it was rejected); the latched fault preserves the trusted state.
The runtime owns the outer schedule passed to `run_lockstep`; live adapters must
never derive it from untrusted timestamps embedded in command payloads.

## Numeric invariants and local outputs

- `CommandEnvelope` and `Constraint` have private numeric configuration.
  `CommandEnvelope::new`, `Constraint::range` and `Constraint::slew_rate` reject
  NaN, either infinity, inverted ranges and negative rates. Equal bounds and zero
  rates are valid. `Limits` retains its validated `Option` constructor with
  private fields and read-only accessors.
- Both gate constructors return `Result`. A safe output must be finite and
  satisfy every configured hard output range. No clamping or neutral-zero
  substitution occurs during construction. Disjoint host ranges cannot create
  a gate because no safe output can satisfy them all.
- Host NaN/infinite commands yield `Rejected(NonFiniteCommand)` before authority
  checking, applying the configured safe value even when no lease exists.
  Direct `LeaseTable::authorize` also rejects non-finite commands. Embedded
  retains authority/link precedence, then `SafeState(BadCommand)` for a
  non-finite request while authority and link are valid.
- Constraint arithmetic is checked. A non-finite intermediate (host rate × time
  or slew endpoints; embedded command delta), or a final non-finite/out-of-range
  output, yields `InvalidOutput` and immediate local fallback. This conservative
  policy can reject an extreme finite request whose intermediate overflows;
  it never treats overflow as unlimited slew. Every final normal output is
  checked against all configured hard ranges after ordered constraints.
- Lease command envelopes govern the requested command, as before. Hard output
  ranges are host range constraints / embedded `Limits`; fallback is validated
  against those ranges and does not require a lease. Configure a suitable local
  safe value for the actual actuator; construction does not establish physical
  safety or hardware suitability.

## API migration

| Previous API | A04.1 API |
|---|---|
| `SafetyGate::new(...) -> SafetyGate` | `-> Result<SafetyGate, SafetyError>`; handle `InvalidSafeOutput`. |
| `gate.evaluate(request)` | `gate.evaluate(request, runtime_now)`; source timestamp stays in the request. |
| `CommandEnvelope { min, max }` / field mutation | `CommandEnvelope::new(min, max)?`, `min()` and `max()` read-only accessors. |
| Public `Constraint::Range` / `Constraint::SlewRate` variants | Private representation; use validated `range` / `slew_rate` constructors and `id()` / `permits_output()`. |
| `Constraint::apply(...) -> f64` | `-> Option<f64>`; `None` means invalid input/arithmetic, not a usable output. |
| Public embedded `Limits` fields | `Limits::new(...).ok_or(...)` plus `min()`, `max()`, `max_step()`. |
| `CommandGate::new(...) -> CommandGate` | `-> Result<CommandGate, GateConfigError>`; invalid safe output is an explicit error. |
| Exhaustive denial/rejection matches | Handle the additional typed numeric and evaluation-clock reasons. |

All workspace callers, executable examples, doctests and lineage test callers are
updated. `SafetyDecision.at` now means evaluation time; consumers of recorded
lineage consequently get evaluation decision time. The source timestamp remains
available in the originating request; no recording schema or freshness protocol
is introduced here. First-command slew remains a no-op on both paths.

## Regression and verification evidence

Regression suites are `crates/safety/tests/gate.rs` and
`crates/embedded-core/tests/behaviour.rs`, with compile-fail doctests preventing
unchecked configuration. They cover delayed commands after lease expiry,
`TickContext` integration, source/evaluation separation, equal times, cross-domain
leases, latched regression and overflow, non-finite configuration/commands,
explicit safe-output rejection, recovery from safe output, extreme arithmetic,
constraint ordering and final hard bounds. Existing replay, lineage, transport
and examples remain in the workspace checks.

Verification results will be recorded here after the branch CI completes. The
local editor environment has no Cargo/rustup or AVR toolchain, and network access
to install Rust is unavailable; it is not reported as a passing local build.
The existing GitHub Actions workflow uses pinned Rust **1.94.1**, locked
dependencies, required C++/Python tools and the separate AVR compile/link job.
The host job additionally checks `neuradix-time` and `neuradix-embedded-core`
with `--no-default-features` to exercise the no_std configuration independently
of workspace feature unification.

## Remaining A04 work and limits

Source age/future skew, command sequence, lease epoch/reboot, deadlines,
host/MCU physical-units-per-time slew alignment, changing-period conformance and
measured rig-specific safe response remain open. No new transport, Studio,
simulation backend, enterprise subsystem or hardware trial is part of A04.1.
Host tests and AVR compilation are not physical hardware validation, real-time
qualification or a certification claim. Other Gate A work packages remain open.
