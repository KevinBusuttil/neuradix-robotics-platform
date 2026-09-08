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

Verified implementation commit: [`9b9d2b2`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/9b9d2b2f92d88b4f3fdae8ff10128fa204498af5).
[Branch CI run 34278834161](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34278834161)
and [PR CI run 34278839451](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34278839451)
both passed. [PR #8](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/8)
tracks integration; subsequent evidence-only edits do not change the tested code.

| Check | Result and evidence |
|---|---|
| Pinned toolchain | Rust 1.94.1 (`e408947bf`), Cargo 1.94.1 (`29ea6fb6a`), locked dependencies; CI uses `RUSTFLAGS="-D warnings"`. |
| Format | `cargo fmt --all --check` passed. |
| Lint / all callers | `cargo clippy --locked --workspace --all-targets -- -D warnings` passed, including all executable examples. |
| Focused regressions | `cargo test --locked -p neuradix-safety -p neuradix-embedded-core -p neuradix-embedded-transport`: **64 passed, 0 failed, 0 ignored**, including three compile-fail configuration checks. |
| Workspace | `cargo test --locked --workspace`: **211 passed, 0 failed, 2 ignored**. The two ignored tests are the explicitly executed AVR checks below, not omitted evidence. Focused tests are a subset, not an additional 64 unique tests. |
| Embedded no_std | Separate `cargo check --locked -p neuradix-time --no-default-features` and `cargo check --locked -p neuradix-embedded-core --no-default-features` passed, independently of workspace feature unification. Host target configuration compilation, not MCU execution. |
| Documentation | `cargo doc --locked --workspace --no-deps` passed; local checks verified **299 relative Markdown paths/anchors**, and `git diff --check` passed. |
| Host conformance prerequisites | Required g++ 13.3.0 and Python 3.12.3 were present; the workspace's C++ and Python tests executed. |
| AVR conformance | `cargo test --locked -p neuradix-embedded-codegen --test avr -- --ignored --nocapture`: **2 passed** with AVR GCC 7.3.0. Supported ATmega328P scalar harness compiled/linked; binary64 projection failed with the required ABI diagnostic. The unchanged codec harness reports 2,778 text + 322 data bytes (3,100 flash), 6 bss bytes (328 static SRAM). This is codec evidence, not an embedded gate footprint or board trial. |
| Earlier failed checks | [Initial run](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34278260262) found formatting differences; corrected using pinned rustfmt. [Second run](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34278634389) found two missing exhaustive matches in the embedded example; corrected. The verified implementation has no remaining failed checks. |
| Unavailable / not run | Local Cargo/rustup and AVR tools were absent; local Rust installation network access was unavailable. Builds and executable tests therefore ran in repository CI. No physical Arduino/MCU, HIL rig, stack measurement or hardware timing test was run. |

The existing CI workflow is preserved, with explicit focused regressions and
independent no_std configuration checks added. Formatting, lint, workspace,
documentation and the separate AVR job remain required evidence in this flow.

## Remaining A04 work and limits

Source age/future skew, command sequence, lease epoch/reboot, deadlines,
host/MCU physical-units-per-time slew alignment, changing-period conformance and
measured rig-specific safe response remain open. No new transport, Studio,
simulation backend, enterprise subsystem or hardware trial is part of A04.1.
Host tests and AVR compilation are not physical hardware validation, real-time
qualification or a certification claim. Other Gate A work packages remain open.
