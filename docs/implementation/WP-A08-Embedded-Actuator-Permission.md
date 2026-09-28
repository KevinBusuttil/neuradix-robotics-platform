# WP-A08 — Embedded actuator-driver permission boundary

Status: **integrated.** [PR #22](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/22) merged as `ecc510e4f2581bd82ee551bf1e3a802c009b00ed` on 28 September 2026 with a tree
identical to reviewed head `31f0300`; [post-merge main CI 36478100811](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36478100811) passed.
WP-A08 remains partial; Gate A remains open. This increment makes no hardware
support, physical safety qualification or Gate A completion claim.

## Verified baseline and scope

Main `620b1964a68f5d0a4a5823bc65201062e881bcbb` was re-verified before work:
[CI 36463845183](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36463845183)
passed host and Arduino Uno codec ABI jobs. PRs #17, #18 and #20 are integrated
(merge commits on main); #21 was documentation only. No open PR owned this
increment, and no AGENTS.md/CLAUDE.md applies. Work is on an isolated branch.

The [host boundary](WP-A08-Actuator-Permission-Binding.md) (PR #20) owns a driver
behind `neuradix_safety::actuator`. Before this increment the embedded
`PropulsionNode::tick` returned a gate-selected `f32` that callers could send
anywhere, and its documentation described that value as "applied to the
actuator". No embedded component owned a permission-guarded driver.

This increment adds `neuradix_embedded_core::actuator`, the `no_std`,
allocation-free equivalent. It reuses the unchanged `CommandGate` and
`neuradix-command-core` validity/slew semantics (A04.1–A04.3). It adds no
executor, board HAL, transport, firmware, graph executor, authentication,
networking, Manufacturing logic or third-party dependency.

## API ownership boundary

| Holder | Receives | Can do |
|---|---|---|
| Board support (trusted) | Implements `ActuatorDriver` | Report fixed endpoint/mode; perform one bounded write |
| Trusted setup/executor | `ActuatorAdapter<D>` (owns driver and gate) | Construct, `grant`, `renew`, `revoke`, `shutdown`, create ports with trusted mode and time |
| Component | Single-use `ActuatorPort` | Submit `Option<Command>` once |

- `ActuatorBinding` = holder (`u64`), capability (`u64`), endpoint (`u64`) and
  `ExecutionMode`, fixed for the adapter's lifetime. Numeric IDs are
  deployment-owned, not credentials; a binding value grants nothing.
- `ActuatorAdapter::new` validates the safe output against `Limits` and checks
  the driver's fixed endpoint/mode descriptor (not hardware attestation). It
  performs no I/O. On error the driver is dropped without a write.
- `DriverPermission::new(binding, lease)` checks the lease holder/capability and
  that the lease timeline's clock domain matches the mode (Live → Monotonic,
  Simulation → Simulation, Replay → Replay). Fields are private; no `Clone`.
- The adapter has no driver/gate accessor, is not `Clone`/`Copy`, and has no
  dispatch taking a `GateDecision` or raw scalar.
- `ActuatorPort` fields are private; `tick(self, Option<Command>)` consumes the
  port. Components cannot choose evaluation time, change mode, install
  permission or submit an output.
- Command payloads carry only the existing `CommandMeta`; they cannot select
  trusted time, generation installation or execution mode. A Replay/Simulation
  port against a Live endpoint (or vice versa) is refused and safes without
  consuming a sequence. A command whose timeline differs from the lease's is
  rejected by the unchanged gate (`UnsupportedClockRelationship`).

Rust ownership does not isolate `unsafe`/native code, a second peripheral handle
obtained elsewhere (for example by stealing HAL singletons), or DMA/interrupt
writers. Deployment owners must ensure the adapter holds the only path to the
output. Source authentication remains out of scope; A04 generation/sequence
checks are replay checks, not authentication.

Moving `ExecutionMode` into `neuradix-command-core` lets both boundaries share one
vocabulary; `neuradix_safety::actuator::ExecutionMode` is a re-export with the
same variants and `clock_domain()`, so host code is source-compatible.

## Selected output, driver acknowledgement and physical state

Every operation returns a fixed-size `Copy` `DispatchReport`:

1. **Gate selection**: `output`, `decision: Option<GateDecision>`,
   `permission`, `evaluation_fault`.
2. **Driver call and acknowledgement**: `write_result` and `fallback_result`
   (`None` means no call). `Ok` acknowledges the call only.
3. **Physical state**: never observed or claimed. A failed write may have had an
   unknown physical effect; a failed safe write is not a physical safe state.

`GateDecision`, `Outcome` and `EmbeddedComponent::tick` documentation now say
"selected" rather than "applied". The `applied` field keeps its name for
compatibility; its documentation states it is not a driver acknowledgement.

## Lifecycle and evaluation

- **Initialization.** Construction performs no I/O. Before a grant, every port
  tick (including with a valid-looking command) reports `Missing` and writes only
  the safe output. Runtime time observed before the first grant is carried into
  the gate, so a grant cannot rewind the evaluation clock.
- **Installation/replacement.** `grant` requires: not shut down, no latched
  driver fault, identical binding, control time in the mode's domain that does
  not regress the gate clock, and (after the first grant) a strictly greater
  generation — also after revocation. Failures change no state and perform no
  I/O. Success writes the safe output immediately and makes it the slew
  reference, so the next command is rate-limited from safe (including at zero
  elapsed time).
- **Renewal** extends an active lease only; sequence, freshness, watchdog, slew
  and clock state are preserved. Renewal of a missing, revoked or expired lease
  fails.
- **Revocation** writes the safe output immediately and retains the generation
  watermark.
- **Idle watchdog.** Periodic `tick(None)` enforces lease expiry, deadline,
  source age and the accepted-command watchdog (A04.2) and selects safe output
  when any fails.
- **Clock faults** (domain change, regression, overflow) latch in the gate and
  cannot be cleared by grant, renewal or revocation.
- **Inhibited paths** (shutdown, driver fault, mode mismatch, missing/revoked
  permission) observe runtime time, select safe output, clear the held output and
  never consume a sequence or refresh liveness.

## Driver failure and shutdown bounds

- Driver errors: `Unavailable`, `Rejected`, `UnknownOutcome`.
- A failed first write latches the first error (never overwritten) and makes
  exactly **one** immediate safe-output fallback. The requested output is never
  retried. Later operations attempt only the safe output; grants/renewals are
  refused until trusted reconstruction with a durably newer generation.
- Every operation makes at most **two** driver calls; `DispatchReport::driver_calls`
  exposes the count.
- `shutdown` is terminal: the first call attempts the safe output (plus at most
  one fallback); later calls, revocations and ports make no driver call. `Drop`
  performs no I/O.
- `ActuatorDriver::write` must return within the deployment's admitted budget;
  the synchronous API cannot bound a blocking implementation. External
  watchdogs and independent device protections remain required.

## Memory bounds

No heap: `neuradix-embedded-core` is `#![no_std]` without `alloc`. State is one
binding, one `Limits`, one safe value, `Option<CommandGate>` (one lease slot),
one pre-grant clock, one status, one latched error and one generation. There is
no queue or history. Measured `size_of` (driver included in the adapter):

| Target | `ActuatorAdapter<DutyCycleDriver>` | `DispatchReport` |
|---|---|---|
| x86_64 host (test driver) | 576 B | 288 B |
| thumbv6m / thumbv7em / riscv32imc | 464 B | 232 B |

A compile-time assertion bounds them at 640/320 bytes. Stack use and timing on a
real board are not measured.

## Migration

Additive. `CommandGate`, `PropulsionNode`, `EmbeddedComponent`, `Limits`,
wire/codec formats, host `SafetyGate`/actuator APIs and graph identities are
unchanged in behaviour. Changes:

- New `neuradix_embedded_core::actuator` module and root re-exports:
  `ActuatorAdapter`, `ActuatorBinding`, `ActuatorDriver`, `ActuatorPort`,
  `DispatchReport`, `DriverError`, `DriverPermission`, `PermissionError`,
  `PermissionStatus`, `ExecutionMode`.
- `ExecutionMode` now lives in `neuradix-command-core`; the host path re-exports
  it (and gains `Hash`).
- `PropulsionNode::tick`'s scalar is documented as a gate selection, not permission
  or actuation. Firmware that sends it to a driver should migrate: give the
  driver to `ActuatorAdapter`, install a `DriverPermission` from trusted
  startup, hand components ports, and remove any remaining raw driver handle.
  Existing raw integrations are not automatically protected.
- `examples/embedded-propulsion` now drives the adapter in Simulation mode with
  an in-memory driver and prints selected output separately from driver
  acknowledgement.

## Verification

Local environment: Rust/Cargo 1.94.1 (pinned), `RUSTFLAGS=-D warnings`, locked
dependencies, AVR GCC from Ubuntu packages.

| Check | Result |
|---|---|
| `cargo fmt --all --check`; workspace all-target Clippy, warnings denied | Passed |
| Embedded actuator suite (`crates/embedded-core/tests/actuator.rs`) | 14 passed |
| Thread-local allocation counter across construction, grant, dispatch, fault, mode refusal, renewal, revocation, replacement, shutdown | 1 passed; zero allocations |
| Compile-fail ownership checks (private permission, no permission clone, no driver field, no adapter clone, port mode/time immutable, no `GateDecision` or raw scalar dispatch, single-use port, no grant through port) | 10 passed, plus module example |
| Target-shaped monomorphization test (`neuradix-example-embedded-actuator-target`) | 1 passed |
| Command/safety/embedded/transport regression group (`cargo test` result lines, including per-scenario subprocess runs) | 138 passed; 112 on main at the baseline, +26 new (14 suite, 1 allocation, 11 doctests); none removed |
| Host actuator suite and example (PR #20) | 10 passed; example passed |
| Workspace tests/doctests excluding `neuradix-python` (root container) | 379 passed |
| `neuradix-python` as an unprivileged user | All passed except `privileged_launcher_rejected`, which requires passwordless sudo (CI provides it); Python SDK 6 passed |
| Examples: minimal-depth-stream, auv-depth-sim, embedded-propulsion, guarded_actuator, resolved_identity, delayed_feedback, program_replay | Passed |
| Independent no-default-features checks: time, command-core, embedded-transport, embedded-core, embedded-actuator-target | Passed |
| MCU cross-compilation `tools/ci/mcu_actuator.sh`: thumbv6m-none-eabi, thumbv7em-none-eabihf, riscv32imc-unknown-none-elf (release) | Passed; no heap-allocator symbol references in time/command-core/embedded-core/target objects; negative control (a `Box` in a `no_std` rlib) is detected |
| Monomorphized code size (object, not linked firmware), thumbv7em | `install` 730 B, `control_step` 164 B, `shutdown` 106 B, `setup` 96 B, `revoke` 72 B |
| Actual ATmega328P codec ABI conformance (`avr` test, ignored by default) | 2 passed locally |
| `cargo doc --workspace --no-deps` | Passed |

The root container causes the Python launcher to refuse privileged execution by
design, so `neuradix-python` was rerun as an unprivileged user; this is a tool
limit, not a code result. CI runs as a non-root user with passwordless sudo.

CI adds an "Embedded actuator permission regressions and example" step, the
target crate to the no_std checks, and a separate **MCU actuator
cross-compilation** job. Remote results, all three jobs (host, Arduino Uno codec
ABI, MCU actuator cross-compilation) passing in each run:

| Commit | Run | Result |
|---|---|---|
| `42f16e8` implementation | [push CI 36467944830](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36467944830) | Passed; MCU job built all three targets from clean and found no allocator references |
| `31f0300` reviewed head | [push CI 36468133448](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36468133448) and [PR CI 36468189893](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36468189893) | Passed |
| `ecc510e` merge on main | [post-merge main CI 36478100811](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36478100811) | Passed |

The Codex review of `31f0300` completed with no findings; no review threads
were opened.

### Not established

- **No hardware execution.** Cross-compilation and host tests do not execute
  this adapter on any MCU. No board, HAL driver, interrupt, DMA or reset path was
  exercised; stack margin, write latency and physical response are unmeasured.
- **AVR.** Rust AVR targets need a nightly toolchain and are not built. The AVR
  job verifies the generated C/C++ codec ABI only, not this adapter.
- **Physical safety.** No qualification, independent protection or safe-state
  measurement is claimed.

## Remaining WP-A08 and Gate A acceptance

WP-A08 remains partial. Its acceptance line "a component labelled Safety cannot
acquire actuator access from its label alone" is now demonstrated for the host
and embedded software boundaries. Remaining A08 work:

1. Connect graph-level roles to trusted binding in the executable graph path
   (B02/B07 dependency), so resolved deployments produce `ActuatorBinding`s and
   grants only through trusted setup; add graph-to-adapter conformance tests.
2. Durable, non-reused generation allocation at startup — **partial**: the
   portable allocator, strict reserved-permission hooks and a Linux host store are
   proposed in [WP-A04.4](WP-A04.4-Generation-Reservation.md); a board `ReservationStore` meeting contract C1–C8, with
   fault-suite qualification, remains (B04/B05).
3. Board-level qualification of one driver on the chosen board (B04/B05):
   exclusive peripheral ownership, bounded write latency, reset/watchdog
   behaviour and measured stack — with the rig documenting its actual safe
   response (shared with A04/ACC-05).

Remaining Gate A acceptance recorded in Capability Status (unchanged by this
increment):

- **A01/ACC-01** — broader evidence inventory and optional-tool audit.
- **A02/ACC-03** — transport binding, compact-ID collision handling and a
  recording migration fixture.
- **A03/ACC-02** — board-generated payloads matching host golden vectors on the
  actual Uno; stack/timing.
- **A04/ACC-05** — trusted startup/board integration, reboot/lease-epoch evidence
  and a rig documenting its physical safe response.
- **A05/ACC-09** — aggregate process-tree resources and additional OS/deployment
  containment.
- **A06/ACC-08** — broader interchange and scale qualification.
- **A07/ACC-07** — closed-loop replay evidence.
- **A08** — items 1–3 above.

**Next recommended task** (Gate A, not Gate B/Manufacturing): the A04/A08
trusted-startup increment — a portable, allocation-free durable generation
reservation interface with host fault-injection tests (power loss between
reserve and use, duplicate/rolled-back storage), used by both actuator
boundaries' setup. Board qualification follows once a board package exists;
Manufacturing remains a future consumer and does not reorder this plan.
That increment is now proposed in [WP-A04.4](WP-A04.4-Generation-Reservation.md).
