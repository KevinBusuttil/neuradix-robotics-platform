# RFC-0020 — Deterministic Vehicle Simulation

- Status: Partially implemented (foundation increment 10)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §10 (simulation), §3.4 (determinism); [Implementation Plan v0.3](../Neuradix_Implementation_Plan_v0.3.md) Phase 3 (single AUV vertical slice); complements RFC-0003, RFC-0005, RFC-0016
- Crate: `neuradix-sim`; example: `examples/auv-depth-sim`

## Problem

The platform proves determinism, control and safety *in the open* — a controller
turns depth samples into thrust, and the safety gate audits them — but nothing
closes the loop. The thrust commands never move a vehicle, so the samples driving
the controller are a canned sequence, not the consequence of the vehicle's own
motion. Phase 3 of the plan ("minimal AUV simulation … depth control … thruster
request … deterministic replay") requires a *simulated plant* the control and
safety path can actually drive, without sacrificing determinism.

## Scope

Implemented in this increment: a fixed-step vertical-depth plant, a deterministic
depth sensor, a narrow controller seam, and a closed-loop driver producing a
reproducible trajectory; plus a worked example that drives the plant through the
real control law **and safety gate**. Out of scope for this increment: multi-axis
/ 6-DOF dynamics, hydrodynamic fidelity, environmental disturbances (currents),
multi-vehicle simulation (RFC-0010), sensor noise models beyond bias +
quantization, and a `neuradix sim` CLI command.

## Proposed decision

### A pure, fixed-step plant

`DepthPlant` models one vertical axis as a point mass. Depth is metres, positive
**downward** (`0.0` = surface); the state is `(depth, velocity)`. Each step
integrates `accel = thrust_accel·u − buoyancy_accel − drag·v` with semi-implicit
Euler (velocity first, then position) over a fixed `dt`, and clamps depth to
`[0, max_depth]`, zeroing velocity at a boundary so the vehicle neither breaches
the surface nor tunnels through the floor. There is no randomness and no ambient
time: the model is a pure function of parameters, state and command.

### A deterministic sensor

`DepthSensor` observes the true depth and returns a *measured* depth, modelling a
constant `bias` and finite resolution (`quantum`). It is noise-free by design, so
the closed loop stays reproducible; the ideal sensor is the identity.

### A narrow controller seam

`Controller::command(&mut self, ctx, measured_depth) -> f64` is the only coupling
between the simulation and the control/safety stack. Because it is this narrow,
`neuradix-sim` depends on **nothing but `neuradix-time`** — a real control law, a
safety-gated law, or a replayed law all implement it without pulling the runtime,
safety or transport crates into the model. A built-in `ProportionalController`
ships for tests and simple runs.

### A deterministic driver

`Simulation` holds an injected `ManualClock` and steps sensor → controller →
plant, advancing the clock by exactly one `dt` per step and recording a
`TrajectorySample` (time, true/measured depth, velocity, applied thrust). Two
identical runs produce a byte-identical `Trajectory` — the same determinism
contract the recorder/replayer rely on (RFC-0003). The step must be strictly
positive; the clock domain flows from the start timestamp onto every sample.

## Public interfaces affected

`neuradix-sim`: `DepthPlant`/`PlantParams`/`PlantState`, `DepthSensor`/
`SensorParams`, `Controller`/`ProportionalController`/`StepContext`,
`Simulation`/`Trajectory`/`TrajectorySample`, `SimError`. The crate depends only
on `neuradix-time`, so it sits beside the other leaves in the layering. No
existing interface changes.

## Alternatives considered

- **Model the plant as a runtime `Processor`.** Rejected for the *plant*: a
  closed loop is feedback (plant output feeds the controller feeds the plant),
  not a pre-baked input sequence, which is what `run_lockstep` consumes. The
  controller seam captures the coupling instead, and the driver owns the loop.
- **Depend on `neuradix-safety` so the gate lives inside the simulation.**
  Rejected: it would make the model non-neutral. The `Controller` seam lets the
  example compose control + safety *around* the plant while the crate stays a
  leaf.
- **Continuous / adaptive-step integration (RK4, variable dt).** Deferred: fixed
  semi-implicit Euler is deterministic, cheap and adequate for a control demo;
  higher-order integrators can be added behind the same `step` contract.

## Safety and security implications

The example demonstrates the safety property that matters: every command reaches
the plant *only* through `SafetyGate::evaluate`, so the range/slew constraints
clamp the raw control law before it can move the simulated vehicle — the same
path a real actuator command takes. Determinism means a mission (and any safety
rejection within it) is exactly reproducible for audit and regression.

## Compatibility implications

`PlantParams`/`SensorParams` may gain fields with `Default`s additively.
`TrajectorySample` may gain fields; consumers read by name. Higher-DOF plants are
new types beside `DepthPlant`, not changes to it.

## Testing strategy

`crates/sim/tests/simulation.rs` covers determinism (two identical runs are
equal), proportional convergence to a setpoint, monotonic descent under constant
thrust, a buoyant vehicle rising and clamping at the surface, floor clamping,
sensor bias/quantization, parameter and step validation, and exact timestamp
advance. `examples/auv-depth-sim` runs the full closed loop through the safety
gate and asserts both convergence and byte-identical re-runs via a content
digest.

## Unresolved questions

- A `neuradix sim run <config>` CLI command and a YAML mission/plant config.
- Recording a simulated mission as a `.nrec` and replaying it through the same
  controller (joins this RFC to RFC-0015).
- Sensor noise (seeded, deterministic) and environmental disturbances.
- Multi-axis / 6-DOF dynamics and multi-vehicle simulation (RFC-0010).
