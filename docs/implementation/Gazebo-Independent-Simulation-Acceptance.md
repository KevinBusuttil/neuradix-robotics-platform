# Gazebo-independent reference simulation: mandatory acceptance contract

**Decision date:** 6 November 2026. **Status:** Approved scope direction; implementation and qualification pending.

Authority: [Specification v0.6](../Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) NRX-SIM-005/006; [Plan v0.4](../Neuradix_Implementation_Plan_v0.4.md) C01–C06 and ACC-06/07/10. This contract defines future acceptance, not executed tests. Gate A/B prerequisites remain unchanged.

## Scope and decisions before implementation

The platform lead and test/control owner must record one reference application, named physical rig and boards, supported host/engine matrix, operating envelope, required geometry/physics/sensors/actuators, calibration method, workloads, resource/timing budgets, repeat counts and numerical/physical error thresholds. Review these before implementation and before trials; changes require a recorded rationale and renewed affected evidence. No unfilled threshold counts as a pass.

The current motor/sensor rig or small mobile robot default is preserved. An AUV is not selected by this document. If selected, required buoyancy, hydrodynamics, thrusters, environment and underwater sensors must have explicit acceptance cases. Broader marine/domain features remain separately scoped. The existing one-dimensional AUV example is a regression fixture, not the completed reference simulator.

## Milestone 5: delivery (C01/C02)

- Neuradix owns world/model loading, scenario orchestration, device bindings, clock/step/reset semantics and evidence output through versioned interfaces.
- The selected workflow builds and executes without Gazebo libraries, processes, services or an installed Gazebo distribution. Audit required dependencies, including transitive dependencies, and runtime processes. Third-party physics engines and independent format parsers are permitted if they preserve this boundary; do not mandate a new physics solver.
- Load the reference robot/environment with required physics, sensors and actuators. Pin engine, models, solver configuration, calibration and random seeds in run identity. Unsupported import/model properties produce explicit diagnostics.
- Automate clean-environment build/run, pinned offline run, reset/repeat, stepping/clock, sensor/actuator contract, malformed/unsupported configuration and selected fault scenarios. Use declared numerical tolerance where bitwise equality is not qualified.
- Retain commands, environment/dependency inventory, artifacts, logs and comparisons. An optional Gazebo adapter or launch wrapper cannot pass these cases. No general Gazebo feature-parity claim follows.

## Milestone 6: integrated workflow (C03/C04)

Through shared CLI/Studio services, configure the reference scene and bindings, validate, run/pause/step/reset and inspect signals, frames, state and fault outcomes without Gazebo. Record visible live/simulation/replay distinctions and active artifact/model identity. UI failure must preserve existing local-control requirements. Retain the documented engineer workflow and ACC-10 results.

## Milestone 7: qualification (C05/C06)

- Run software simulation, actual program replay and HIL against the named Uno/MCU/host matrix. Preserve common component contracts and distinguish virtual, replayed and physical state.
- Compare simulation with calibrated physical measurements within the predeclared operating envelope; report model error, timing, jitter, sensor/actuator response and applicable resource budgets separately from interface conformance.
- Execute the declared communication, sensor, process and clock fault matrix; measure physical fallback where required. Record failures as well as successful trials.
- Retain seed/reset repeatability evidence, exact executable/configuration/model identities, raw traces, calibration provenance, repeat counts, numerical and physical comparison results, hardware identities and known limitations.
- Program replay must execute the selected program; logged observations alone do not prove changed physical outcomes. Host tests, cross-compilation and simulated drivers do not establish physical qualification.

Milestone 7 and Gate C remain open until all applicable ACC-06/07/10 and other Gate C criteria pass. Missing hardware, unresolved tolerances or unavailable evidence is an explicit blocker. Milestone 5 software delivery alone does not close Gate C or establish commercial support.

## Planning and reporting

RFC-0025 must record backend alternatives, licensing/dependency considerations, supported import subset, service boundary and failure/timing semantics before public interfaces stabilise. Re-estimate C01/C02 and downstream effort after the backend spike; previous Gazebo-first estimates are not validated for this scope. Update Capability Status only with exact demonstrated evidence. Preserve optional Gazebo/ROS interoperability as separately qualified adoption features.
