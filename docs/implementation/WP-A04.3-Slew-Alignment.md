# WP-A04.3: host/MCU slew alignment and changing-period conformance

This increment implements the remaining software slew portion of WP-A04,
**Trusted command evaluation and finite limits**. WP-A04 remains **partial**,
ACC-05 remains incomplete and **Gate A remains open**: measured rig response,
board scheduling, durable startup and physical reset/timing evidence are missing.
Local control and safing remain independent of AI, cloud, UI and transport services.

## Reviewed baseline and prerequisite corrections

PR #9 was open at `48dde8ba`, with passing CI and two subsequent automated review
findings. Both were confirmed and corrected before integration:

- Delayed renewal could supply time older than an observed lease expiry and revive
  the same generation. Renewal now checks the owning gate's latest evaluation
  clock and fault state. Host renewal is `SafetyGate::renew_lease`; core session
  renewal requires the owning `EvaluationClock`. Stale, incompatible or faulted
  renewal time returns `InvalidEvaluationTime`, without changing the lease.
- CLI explanations lost the new non-finite request markers, and numeric series
  could aggregate them. Explanations preserve all three markers; selecting a
  non-finite series field returns an explicit decode error with field/time.
  Finite applied-output series remain available. This corrects existing readers.

Corrected head [`665fd5d`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/665fd5d285d4ba92dd5771bf0ab40ff0cd1b8f56)
passed [CI 34288937294](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34288937294):
71 focused and 219 workspace tests/doctests, two AVR checks, formatting, Clippy,
three examples, four independent no_std checks and documentation build. An initial
format failure was corrected using the pinned formatter. No required external
review or ruleset blocked integration. [PR #9](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/9)
was merged as [`c127c7d`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/c127c7d433b8e5a07f7a7df4b5a96cf6e6ed7a03).
A04.3 starts from that main revision on `codex/a04-3-slew-alignment`, in an isolated
checkout. No applicable AGENTS.md was found; existing checkouts/branches were preserved.

## Defect and resulting behavior

The host limited output change in units per second, while MCU `max_step` limited
change per evaluation. More frequent or repeated same-time evaluation could move
the MCU actuator faster without elapsed runtime time. `neuradix-command-core`
now provides one validated `SlewRate` implementation used by both gates. It has
no allocations, std, executor or new dependency. No freshness, authority,
sequence, generation, deadline or watchdog policy is relaxed for slew processing.

| Event | Policy on both paths |
|---|---|
| New valid command | Constrain from the previous **applied** output over `now - previous_evaluation_time`. Budget is `rate_per_second * elapsed_seconds`. Final output remains finite and within all hard ranges. |
| Source time | Used only by A04.2 source validity. It never supplies elapsed slew time. Processor evaluation still uses `TickContext.now`. |
| First evaluation with valid command | No prior output reference: apply hard ranges only. This preserves the explicit A04.1 initialization exception, including a zero rate. |
| First evaluation without valid command | Apply configured safe output and establish the reference. The first subsequently valid command is slew-limited from that safe output. |
| Equal evaluation time / zero rate | Hold the established output exactly. Multiple commands at the same timestamp gain no movement allowance. Fully valid commands still commit A04.2 sequence/liveness state. |
| Idle tick | Check held-command validity/expiry, then hold the previously applied output. Do not ramp toward a stored target, refresh liveness or accumulate unused slew budget. Advance valid evaluation time. |
| Rejection / expiry | Apply validated local safe output immediately, bypassing slew. Rejections never feed accepted-command liveness. Every subsequent valid evaluation uses the last applied safe reference. |
| Renewal / generation replacement | Preserve output and evaluation-clock reference. Replacing a session does not grant a new initialization exception. A held command from a replaced generation still safes at the next idle evaluation. |
| Clock domain change / regression / elapsed overflow | Preserve the A04.2 latched runtime fault and safe output across all subsequent evaluations and session changes. |
| Numeric overflow | Reject `InvalidOutput` on a non-finite binary64 budget/interval or invalid final output. Overflow never means unlimited slew and does not commit sequence or liveness. |

This is a command gate, not a continuously running target interpolator. For
constant-target progression, the caller supplies a valid new command at each
intended update. Missing input causes a hold, so idle time is not banked for the
next command. Runtime must keep calling `evaluate(None, now)` on its bounded
schedule and apply each returned output; A04.2's expiry and scheduling obligations
remain. Host suspension/simulation pause cannot stand in for a hardware watchdog.

## Numeric representation and tolerances

Rates, bounds and safe values are immutable after validated construction. Rates
must be finite and nonnegative; hard bounds finite and ordered. Embedded rate
configuration and commands remain `f32`; rates/values widen exactly to `f64` for
the shared budget/interval calculation. Host configuration/output remain `f64`.
For conformance, the host receives the exact widened embedded rate/bounds/input.

Elapsed nanoseconds are checked in integer time first, then converted to binary64
seconds for `rate * (nanoseconds / 1e9)`. Non-finite intermediate budgets and
endpoints reject. Binary64 endpoint rounding is corrected inward if its computed
change exceeds the computed budget. Binary32 output then rounds **toward the
previous output**, so output quantization cannot spend more than that allowance.
Underflow or a sub-ULP budget may hold output. No fractional budget is retained.

For ordinary shared trajectories, tests bound accumulated host/MCU difference by
`2 * N * f32::EPSILON * S`, where `N` is evaluated ticks and `S` the largest visited
output magnitude or 1, whichever is larger. Independent per-step rate assertions
allow only binary64 arithmetic tolerance `8 * f64::EPSILON * max(S, budget)`;
zero-time/zero-rate and idle holds require exact equality. First-evaluation
initialization and immediate safing are explicitly exempt from slew bounds.

Equal total elapsed time with different partitions gives equivalent constant-target
progress within this representation bound. It does not promise bit-identical
trajectories: many sub-ULP steps can hold while one longer step can advance.
Subnormal and half-ULP tests document this boundary, including both directions.

Widened MCU arithmetic avoids the old binary32 subtraction overflow between
extreme finite endpoints; if the shared calculation and final output are valid,
such commands can now be accepted. True binary64 overflow still rejects. Binary64
arithmetic on a native MCU may use software routines: execution cost, stack and
timing are **unmeasured**. This does not add or qualify a generated AVR gate.

## API and example migration

| Previous | A04.3 |
|---|---|
| `Limits::new(min, max, max_step)` | `Limits::with_slew_rate(min, max, rate_per_second)`; old constructor removed to prevent silently reinterpreting per-step values. |
| `Limits::max_step()` | `Limits::rate_per_second()` in units per second. |
| Host `Constraint::slew_rate(id, rate)` | Signature retained; delegates to shared `SlewRate`, with inward rounding and the same numeric validation/fallback. |
| Host `LeaseTable::renew(...)` on pre-review A04.2 | `SafetyGate::renew_lease(...)`, guarded by the gate's runtime clock; merged prerequisite correction. |

For migration only, an old step of 0.2 at a declared 20ms period corresponds to
10 units/s. Select the physical rate explicitly; there is no implicit period in
configuration. The 50Hz embedded propulsion example uses 10 units/s. Transport
tests and embedded examples use explicit rates; the A04.2 paired tests use equal
host/MCU rates to isolate validity. No serial payload or framing format changes.
Standalone Watchdog API and all A04.2 trusted-startup requirements remain intact.

## Verification evidence

Branch verification is in progress. Final pinned-toolchain CI identities, passed,
failed and unavailable checks will be recorded before requesting PR review.

Focused additions are `crates/safety/tests/slew_conformance.rs` (both actual gates)
and `crates/command-core/tests/slew.rs` (representation/configuration boundaries).
They cover partitioned time, jitter/direction changes, zero elapsed/rates, first
evaluation, idle holds, immediate safing/recovery, session changes, hard ranges,
source/runtime separation, quantization, extreme finite values and latched clocks.
The migrated A04.1/A04.2 suites, CLI regressions, examples, architecture checks,
workspace/doctests, independent no_std configurations, docs and AVR conformance
remain in the verification path. Compiler checks are not physical hardware validation.

## Remaining work

Measured rig-specific safe response, actual board timer rollover/restart,
durable generation allocation, deployed scheduling and hardware watchdog
integration, and timing/stack/resource evidence remain unqualified. WP-A04 is
partial, ACC-05 is incomplete and Gate A remains open. No board firmware,
storage driver, synchronization protocol, new transport, authentication, Studio
feature, simulation backend, enterprise feature or archived-link repair is added.
