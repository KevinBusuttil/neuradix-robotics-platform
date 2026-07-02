---
title: "Neuradix Robotics Platform — Implementation Plan"
subtitle: "Engineering execution plan aligned to Functional Specification v0.4"
author: "Engineering"
date: "02 July 2026"
version: "0.2 Draft"
status: "For review"
supersedes: "Neuradix_Implementation_Plan_v0.1.md"
related_specification: "Neuradix_Robotics_Platform_Functional_Specification_v0.4.md"
---

# 0. Purpose

This document is the implementation plan for the **Neuradix Robotics Platform**. It is aligned to the **Functional Specification v0.4**, which expands the original platform scope with:

- the platform overview mind map;
- Neuradix Swarm;
- Neuradix Studio XR;
- Neuradix Aero;
- marine AUV swarm support;
- UAV swarm support;
- the Flight and Safety profiles;
- richer domain profiles and operational sub-platforms.

This plan is intentionally more conservative than the functional specification. The specification defines the target platform. This plan defines the practical order in which a small engineering team should build it without collapsing under scope.

The central rule is:

> Build one thin, complete, working vertical slice before widening into Swarm, Aero, Flight, Fleet or rich XR authoring.

The first engineering target remains a **single AUV vertical slice**:

```text
contract → generated Rust/Python APIs → runtime → local transport → simulated AUV
→ record → replay → explain one actuator command → Studio inspection
→ Python crash isolation → safety rejection path
```

Only after that path works end-to-end should the project widen into multi-vehicle swarm behaviour, aerial robotics, mission-scale XR interaction, and flight-profile work.

---

# 1. Executive engineering assessment

## 1.1 What Neuradix is building

Neuradix is not merely a middleware layer. It is a **contract-driven robotic application platform** spanning:

1. **Common Foundations**
   - contracts;
   - runtime;
   - data plane;
   - safety;
   - record/replay;
   - security;
   - observability;
   - time, units and frames;
   - deployment and packaging.

2. **Core Platform Services**
   - Neuradix Ground;
   - Neuradix Fleet;
   - Neuradix Swarm;
   - Neuradix Studio XR;
   - Neuradix Sim;
   - Neuradix Record;
   - Neuradix Bridge;
   - Neuradix Registry.

3. **Execution / Deployment Profiles**
   - Edge;
   - Embedded;
   - Flight;
   - Safety;
   - Ground;
   - Sim.

4. **Domain Profiles**
   - Marine;
   - Aero;
   - Space / Flight;
   - Ground Robotics.

This is an ambitious multi-year programme. A full implementation with production-quality Studio, Swarm, Aero, Flight, Ground/Fleet and simulator capability is realistically a **5–8 year multi-team programme** if taken to the quality bar implied by the specification.

## 1.2 What makes it tractable

The tractable strategy is:

```text
Prove the architectural thesis first.
Add domain breadth later.
```

The architectural thesis is proven when Neuradix can demonstrate all of the following with one simple robot scenario:

- authored semantic contract;
- generated Rust and Python APIs;
- deterministic time model;
- bounded data plane;
- local runtime lifecycle;
- simulator source;
- hardware-capability abstraction;
- recording to MCAP;
- deterministic replay;
- causal explanation of one command;
- safety authority rejection path;
- Python component crash isolation;
- Studio inspection view.

That is the minimum credible product nucleus.

## 1.3 The four load-bearing keystones

These are the highest-risk and hardest-to-change decisions.

| Keystone | Why it matters | Must be proven by |
|---|---|---|
| **K1 Contract system and code generation** | Every SDK, transport, recording, Studio panel, bridge and conformance test depends on it. | Phase 0/1 |
| **K2 Transport-neutral data plane** | Prevents Zenoh, shared memory or DDS from leaking into the public API. | Phase 1 |
| **K3 Time and determinism model** | Replay and explainability fail if components freely read wall-clock time or ambient RNG. | Phase 0/1 |
| **K4 Component model, lifecycle and execution classes** | Safety, supervision and scheduling all depend on the component contract. | Phase 1 |

Everything else depends on these.

---

# 2. Guiding implementation principles

## 2.1 Vertical before horizontal

A capability is not considered implemented until it crosses the full stack.

For example, `DepthMeasurement` is complete only when it exists as:

- a contract file;
- generated Rust type;
- generated Python type hint;
- runtime stream;
- local transport stream;
- recorded MCAP stream;
- replayed stream;
- inspected in Studio;
- validated for time/unit/frame semantics.

## 2.2 Contracts are the source of truth

Protobuf, Python stubs, Rust types, Studio metadata and documentation are generated projections. The authored Neuradix contract file is the source of truth.

## 2.3 Backends are replaceable

The public API must not expose:

- Zenoh types;
- Tokio handles;
- shared-memory internals;
- PyO3 internals;
- DDS-specific QoS semantics.

Backends are implementation choices behind the data-plane API.

## 2.4 Bounded by default

Every queue, pool, task, buffer and stream must declare:

- capacity;
- overflow policy;
- deadline or validity period where applicable;
- priority;
- drop/supersede behaviour;
- telemetry for occupancy and drops.

## 2.5 Determinism is enforced

Determinism is not a coding style. It must be enforced by:

- injected clocks;
- deterministic test harness;
- replay equivalence tests;
- lints banning ambient wall-clock access in deterministic components;
- explicit random seeds;
- no unmanaged threads in deterministic profiles.

## 2.6 Python is first-class but isolated

Python is supported for:

- AI;
- perception experiments;
- mission prototyping;
- analysis;
- simulation scripting.

Python must not be inside:

- hard-real-time control;
- safety authority;
- flight-critical runtime;
- actuator enforcement.

## 2.7 Studio and XR are observers before commanders

Neuradix Studio and Studio XR should first inspect, replay and explain. Commanding and authoring come later and must route through Neuradix Ground authority and onboard Safety.

## 2.8 Flight is a profile, not a marketing claim

Neuradix Flight should begin as a design and simulation profile. It must not be described as certified, human-rated or flight-qualified without mission-specific assurance.

---

# 3. Recommended repository structure

Use a monorepo with a Rust workspace and clear crate layering.

```text
neuradix-robotics-platform/
  Cargo.toml
  rust-toolchain.toml
  deny.toml
  crates/
    contracts/
    time/
    frames/
    transport-api/
    transport-local/
    transport-shm/
    transport-zenoh/
    data-plane/
    runtime/
    safety/
    record/
    sim/
    bridge/
    cli/
    testkit/
  python/
    neuradix/
  studio/
    studio-engine/
    web/
    desktop/
    xr/
  contracts/
    standard/
  examples/
    minimal-robot/
    auv-depth-hold/
    auv-swarm-sim/
    uav-swarm-sim/
  docs/
    rfcs/
    implementation/
    specifications/
  tools/
    codegen/
    ci/
  governance/
```

## 3.1 Crate dependency rules

```text
contracts     → no internal deps
time          → contracts
frames        → contracts, time
transport-api → contracts, time
transport-*   → transport-api
data-plane    → transport-api, contracts, time, frames
runtime       → data-plane, contracts, time
safety        → runtime, contracts, time, frames
record        → contracts, time, frames, data-plane
sim           → runtime, contracts, time, frames, record
bridge        → data-plane, runtime, contracts
cli           → runtime, contracts, record, sim
testkit       → runtime, contracts, time, record
studio-engine → contracts, time, frames, record
python        → runtime, data-plane, contracts
```

`contracts`, `time` and `frames` must not depend on the full runtime. They must remain usable by code generation, Studio, embedded builds and Flight-profile builds.

---

# 4. Implementation tracks

## Track C — Contracts and code generation

**Objective:** create the semantic contract system that defines all Neuradix interfaces.

### Scope

- YAML/TOML authoring format.
- Contract normalization and content-addressed schema ID.
- Units, frames, clock domains and validity metadata.
- Stream, State, Command, Task, Event and Query primitives.
- Rust type generation.
- Python type stub generation.
- Protobuf schema generation.
- Compatibility checker.
- Contract documentation generator.
- Conformance test generator.

### First contracts

Start with a minimal set:

```text
DepthMeasurement
ImuSample
BatteryState
ThrusterCommand
VehiclePose
VehicleHealth
MissionMode
SafetyDecision
ActuatorCommandLineage
```

### Exit criteria

- `DepthMeasurement` can be authored once and projected into Rust, Python and Protobuf.
- Compatibility checker detects a unit mismatch.
- Generated Rust can compile in a minimal component.
- Generated Python type hints are usable by an example Python component.

---

## Track T — Time and frames

**Objective:** make time, clock domains and coordinate frames explicit from the start.

### Scope

- Typed timestamp with clock domain.
- `Clock` trait.
- System clock, simulation clock and replay clock.
- Time synchronization status model.
- Frame identifiers.
- Static and dynamic transforms.
- Transform validity interval.
- Transform uncertainty.
- Frame graph validation.

### Exit criteria

- A component cannot publish a production timestamp without a clock domain.
- Replay can drive a component from a replay clock.
- Studio can display stale data based on contract `maximum_age`.
- Frame mismatch is detected in at least one test.

---

## Track D — Data plane and transport

**Objective:** implement the public transport-neutral communication layer.

### Scope

- Stream.
- State.
- Command.
- Task.
- Event.
- Query.
- In-process transport.
- Local process transport.
- Shared-memory lease model.
- Zenoh backend after the local API is stable.
- Bounded queues and backpressure.
- Payload priority.
- Transport metrics.

### Critical design rule

The public data-plane API must not reveal the chosen backend.

### Exit criteria

- A stream can move `DepthMeasurement` in-process.
- The same stream can be switched to local transport without changing component code.
- A large payload can be represented with a loaned buffer or fallback copy.
- Queue overflow is visible in metrics.

---

## Track R — Runtime and lifecycle

**Objective:** run components under explicit lifecycle and execution policies.

### Scope

- Component manifest.
- Lifecycle states.
- Startup/shutdown ordering.
- Health model.
- Execution class declaration.
- Soft deterministic executor.
- Async service executor.
- Python worker process supervision.
- Restart policy.
- Watchdog.
- Resource telemetry.

### Lifecycle states

```text
Declared
Configured
Inactive
Active
Degraded
Failed
Stopping
Stopped
```

### Exit criteria

- A Rust component can be started, activated, degraded and stopped.
- A Python component can crash without killing the runtime.
- Component restart policy is enforced.
- Health state is visible via the data plane.

---

## Track S — Safety authority

**Objective:** ensure actuator commands pass through explicit authority and constraints.

### Scope

- Command authority model.
- Authority lease.
- Actuator command gate.
- Safety constraints.
- Local safe-state policy.
- Constraint rejection result.
- Safety event.
- Command lineage integration.

### Exit criteria

- A planner command is rejected when it violates a depth/thrust limit.
- The rejection is recorded.
- Studio or CLI can explain the rejection.
- No ordinary component can publish directly to the final actuator interface.

---

## Track REC — Record, replay and explain

**Objective:** make every incident reproducible and explainable.

### Scope

- MCAP recording.
- Contract/schema snapshots.
- Topology manifest snapshot.
- Component/version metadata.
- Clock relationship metadata.
- Safety decision recording.
- Replay clock.
- Deterministic replay harness.
- Branch replay.
- Explain query.

### Exit criteria

- The AUV depth-hold mission can be recorded.
- Replay reproduces the same command sequence under deterministic mode.
- A command can be traced back to sensor input, planner decision, authority and safety result.

---

## Track SIM — Simulation

**Objective:** support simulation-hardware parity.

### Scope

- Simulation clock.
- Minimal AUV dynamics.
- Depth sensor model.
- IMU stub.
- Thruster model.
- Battery model.
- Fault injection.
- Scenario file format.
- SIL test runner.
- Deterministic seeds.

### Exit criteria

- A simulated AUV can run a depth-hold mission.
- A DVL/IMU/depth fault can be injected.
- A failed scenario emits a replayable artifact.
- The same component contract is used by sim and hardware-capability abstraction.

---

## Track PY — Python SDK

**Objective:** provide safe Python integration without compromising runtime integrity.

### Scope

- Python component base class.
- Stream subscription and publishing.
- Typed generated interfaces.
- Process isolation.
- Crash supervision.
- NumPy view support where safe.
- Copy fallback across process boundaries.
- Python package build with Maturin/PyO3.

### Exit criteria

- Python component subscribes to a stream and publishes derived output.
- Python crash is detected.
- Runtime continues running.
- Component health changes to failed/degraded.

---

## Track STD — Studio and Studio XR

**Objective:** provide inspection first, XR later.

The detailed plan is in `Neuradix_Studio_XR_Implementation_Plan_v0.2.md`.

### Initial Studio scope

- open MCAP recording;
- inspect contracts;
- inspect timeline;
- show component graph;
- plot scalar streams;
- show health;
- explain one command;
- display stale data;
- display safety rejection.

### Deferred XR scope

- 3D world view;
- headset rendering;
- spatial commands;
- swarm visualization;
- multi-user authority;
- digital-twin command preview.

---

## Track SWM — Neuradix Swarm

**Objective:** runtime cooperation among multiple robots.

This track is **not** part of the initial core vertical slice. It must be designed early enough to avoid contract rework, but implementation should start after the single-robot foundation is stable.

### Scope

- swarm identity;
- membership epochs;
- roles;
- capabilities;
- task allocation;
- formation/coverage control;
- partition tolerance;
- rejoin reconciliation;
- federated shared world model;
- cooperative localization;
- multi-vehicle safety constraints;
- swarm record/replay.

### First swarm demo

A simulated three-AUV survey mission:

```text
AUV-1 maps sector A.
AUV-2 maps sector B.
AUV-3 acts as acoustic relay.
AUV-2 link drops.
Swarm reallocates pending coverage.
AUV-2 rejoins.
World model and task completion reconcile.
```

### Exit criteria

- Membership epoch changes are recorded.
- A partition and rejoin can be simulated.
- Task assignment is capability-based.
- Local safety overrides a swarm command.

---

## Track AER — Neuradix Aero

**Objective:** aerial robotics domain profile for UAVs and aerial swarms.

This track begins after Swarm primitives exist.

### Scope

- airframe model: multirotor, fixed-wing, VTOL/hybrid;
- airspace world model;
- 3D geofences;
- dynamic exclusion volumes;
- UAV trajectory contract;
- local collision avoidance;
- lost-link behaviour;
- emergency landing/diversion;
- formation control;
- UAV digital twin;
- assisted piloting authority profile;
- headset visualization of airspace and predicted trajectories.

### First Aero demo

A simulated four-UAV inspection mission:

```text
two multirotors inspect a structure;
one fixed-wing UAV maps the wider area;
one UAV acts as relay;
operator defines inspection volume in Studio XR simulation;
collision-avoidance overrides formation path;
mission is replayed and explained.
```

### Exit criteria

- UAVs remain locally safe when swarm/ground link drops.
- Collision avoidance overrides planned formation trajectory.
- Studio XR distinguishes live, predicted, stale and simulated state.
- Emergency landing zone is selected when low battery is injected.

---

## Track FLT — Neuradix Flight

**Objective:** static, deterministic, qualification-oriented execution profile.

This is a long-term track and should not be on the initial v1.0 critical path.

### Scope

- static topology;
- restricted Rust profile;
- no Python in flight-critical paths;
- bounded ports;
- cyclic/rate-group scheduler prototype;
- command/telemetry/events/parameters;
- watchdog;
- FDIR;
- RTEMS or bare-metal target evaluation;
- evidence package generator.

### First Flight demo

A non-flight-certified simulation:

```text
static flight topology;
rate-group scheduler;
telemetry and command dictionaries;
fault injection;
FDIR transition to safe mode;
evidence report emitted by build.
```

### Explicit limitation

Neuradix Flight Alpha is an engineering prototype. It is not certified, human-rated or launcher-qualified.

---

## Track GND/FLEET — Ground and Fleet

**Objective:** mission operations, assets and deployment management.

### Ground scope

- operator identity;
- authority;
- command review;
- command timeline;
- mission procedures;
- telemetry dictionary;
- command dictionary;
- event log;
- audit.

### Fleet scope

- vehicle registry;
- hardware inventory;
- software versions;
- deployment manifests;
- OTA update plan;
- mission assignment;
- health summaries.

### Exit criteria

- Operator command is audited.
- Vehicle software version and manifest hash are visible.
- Fleet can assign mission package to a simulated vehicle.

---

## Track BR — Bridges and interoperability

**Objective:** interoperate without contaminating the core API.

### Priority bridges

1. ROS 2 bridge.
2. MAVLink/MAVSDK bridge.
3. Foxglove WebSocket compatibility.
4. Zenoh backend.
5. DDS boundary adapter.
6. CAN/CANopen adapter.
7. OPC UA/MQTT/gRPC later.

### Rule

Bridges are boundary components. ROS 2, MAVLink, DDS or Foxglove concepts should not become the Neuradix internal model.

---

# 5. Phased roadmap

## Phase 0 — Foundation design and RFCs

**Duration:** 4–6 weeks.

### Deliverables

- workspace skeleton;
- CI;
- governance;
- RFC-0001 Component Model;
- RFC-0002 Contract Format;
- RFC-0003 Time and Determinism;
- RFC-0004 Transport API;
- RFC-0005 Safety Authority;
- initial `contracts` parser spike;
- initial `Clock` trait;
- initial deterministic test harness.

### Do not build yet

- rich GUI;
- XR;
- swarm;
- aerial robotics;
- flight runtime;
- fleet management.

---

## Phase 1 — Minimal contract-to-runtime path

**Duration:** 8–12 weeks.

### Deliverables

- contract parser;
- Rust codegen;
- Python type stub generation;
- in-process stream;
- simple runtime lifecycle;
- deterministic clock injection;
- minimal CLI.

### Demonstration

`DepthMeasurement` stream from one component to another.

---

## Phase 2 — Record/replay/safety nucleus

**Duration:** 10–14 weeks.

### Deliverables

- MCAP recorder;
- replay clock;
- safety authority gate;
- first command lineage;
- Python component isolation;
- local Studio inspection prototype;
- bounded queues;
- health model.

### Demonstration

A simulated component generates a command, Safety modifies/rejects it, and replay reproduces the result.

---

## Phase 3 — Single AUV vertical slice

**Duration:** 12–20 weeks.

### Deliverables

- AUV depth-hold simulation;
- simple dynamics;
- depth sensor;
- thruster command;
- energy model;
- Python AI/perception placeholder;
- replay equivalence;
- command explain;
- Studio inspection;
- bridge stubs.

### Demonstration

A simulated AUV holds target depth, experiences a fault, records the mission, replays it, and explains why a thrust command changed.

This is the recommended **v1.0 alpha target**.

---

## Phase 4 — Marine multi-AUV and Swarm Alpha

**Duration:** 16–24 weeks after Phase 3.

### Deliverables

- Swarm membership;
- membership epochs;
- task allocation;
- basic formation/coverage control;
- partition/rejoin simulation;
- federated world-model records;
- acoustic communication policy simulation;
- multi-AUV mission recording/replay.

### Demonstration

Three simulated AUVs perform cooperative survey with relay role, communication partition and task reallocation.

---

## Phase 5 — Studio XR Alpha

**Duration:** parallel after core Studio engine is stable.

### Deliverables

- 3D scene renderer;
- headset mode;
- live/predicted/simulated/replay state distinction;
- spatial operator intent builder;
- command preview through digital twin;
- multi-user authority model;
- swarm visualization.

### Demonstration

Operator uses headset to define a survey region in simulation. The system generates task allocation, previews it, and records the accepted mission.

---

## Phase 6 — Aero Alpha

**Duration:** after Swarm Alpha.

### Deliverables

- UAV airspace model;
- multirotor simulation;
- fixed-wing abstraction;
- 3D geofences;
- local collision avoidance;
- formation control;
- emergency landing policy;
- Aero Studio XR overlays.

### Demonstration

Four UAVs perform a simulated inspection mission with collision-avoidance override and operator-defined 3D region.

---

## Phase 7 — Ground/Fleet Beta

**Duration:** after Phase 3, can overlap with Phases 4–6 if team size allows.

### Deliverables

- vehicle registry;
- manifest management;
- deployment metadata;
- operator identities;
- command audit;
- health dashboard;
- OTA update plan.

---

## Phase 8 — Flight Alpha

**Duration:** long-term, after core stability.

### Deliverables

- static topology compiler;
- restricted Rust profile;
- cyclic/rate-group scheduler;
- command/telemetry/events/parameters;
- watchdog and FDIR;
- evidence-generation prototype;
- RTOS/bare-metal evaluation.

### Demonstration

A static flight-control simulation enters safe mode after injected sensor fault and emits evidence report.

---

# 6. First 90-day execution plan

## Weeks 1–2

- Create workspace.
- Set Rust toolchain pin.
- Add CI.
- Add governance files.
- Draft RFC-0001 to RFC-0005.
- Create `contracts`, `time`, `transport-api`, `runtime`, `cli`, `testkit`.

## Weeks 3–4

- Implement contract parser spike.
- Define `DepthMeasurement`.
- Generate first Rust type manually or semi-automatically.
- Implement `Clock` trait.
- Create deterministic test harness.
- Decide normalized schema hash.

## Weeks 5–6

- Implement in-process stream.
- Implement component lifecycle skeleton.
- Create two Rust components exchanging `DepthMeasurement`.
- Add CLI `neuradix graph` and `neuradix run` stubs.
- Emit basic metrics.

## Weeks 7–8

- Generate Rust code from contract.
- Add Python type stub generation.
- Add health state.
- Add bounded queue policy.
- Add replay clock skeleton.

## Weeks 9–10

- Add MCAP write path.
- Record a stream.
- Replay the stream.
- Add safety decision contract.
- Add first authority gate.

## Weeks 11–12

- Build minimal simulated depth-hold loop.
- Inject safety rejection.
- Record and replay the scenario.
- Produce first `neuradix explain` CLI output.
- Write Phase 1/2 lessons learned.

---

# 7. RFC backlog

These RFCs should be created before implementation widens.

| RFC | Title | Priority |
|---|---|---|
| RFC-0001 | Component and Lifecycle Model | P0 |
| RFC-0002 | Contract Format and Code Generation | P0 |
| RFC-0003 | Time, Clocks and Deterministic Replay | P0 |
| RFC-0004 | Transport-Neutral Data Plane | P0 |
| RFC-0005 | Safety Authority and Command Lineage | P0 |
| RFC-0006 | Swarm Membership and Task Allocation | P1 |
| RFC-0007 | Federated Shared World Model | P1 |
| RFC-0008 | Studio XR Operator Intent and Authority | P1 |
| RFC-0009 | Aero Airspace and Collision Safety Model | P1 |
| RFC-0010 | Multi-Vehicle Simulation and Replay | P1 |
| RFC-0011 | Flight Profile and Restricted Rust Policy | P2 |
| RFC-0012 | Ground/Fleet Identity and Deployment Manifest | P2 |

---

# 8. Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---:|---:|---|
| Scope expands into Swarm/Aero/XR before the core vertical slice works | High | High | Phase gates; no multi-vehicle implementation before Phase 3 exit |
| Contract model changes after ecosystem begins | Medium | High | RFC review, compatibility engine, schema hashes |
| Zero-copy buffers across Python become unsafe or too complex | Medium | High | Copy fallback for v1; isolate unsafe code |
| Determinism broken by ambient time/thread/RNG | High | High | Lints, testkit, replay equivalence |
| Zenoh or backend semantics leak into public API | Medium | High | API snapshot tests and code review rule |
| Studio becomes a parallel schema implementation | Medium | High | Studio engine links real core crates |
| XR command path bypasses authority | Low | High | Studio commands are semantic intents only; Ground/Safety mandatory |
| Aerial collision avoidance underestimated | Medium | High | Aero deferred until Swarm/core are stable; local avoidance profile |
| Flight profile distracts from robotics platform | High | High | Flight remains paper/prototype until core product maturity |
| Simulator fidelity becomes a separate large project | High | Medium | Start with minimal models; support external simulator bridges |
| Small team cannot staff multiple tracks | High | High | One vertical slice; one active domain expansion at a time |

---

# 9. Version 1.0 alpha acceptance criteria

Neuradix reaches v1.0 alpha when the project can demonstrate:

1. One authored contract generates Rust and Python interfaces.
2. A Rust component publishes a typed stream.
3. A Python component consumes a typed stream and publishes derived output.
4. Runtime supervises both components.
5. Python crash does not crash the runtime.
6. All queues are bounded and emit metrics.
7. Time is injected through explicit clock interface.
8. A depth-hold AUV simulation runs.
9. A safety authority gate modifies or rejects an actuator request.
10. A mission is recorded to MCAP with schema and manifest metadata.
11. Replay reproduces deterministic command output.
12. `neuradix explain` traces one actuator command to source data and safety decision.
13. Studio opens the recording and shows graph/timeline/health/contract metadata.
14. A bridge boundary example exists for ROS 2 or MAVLink.
15. Documentation includes public instructions to reproduce the demo.

---

# 10. What not to build before v1.0 alpha

Do not build the following before the single-AUV vertical slice is complete:

- complete Swarm runtime;
- complete Aero profile;
- production XR headset command interface;
- flight runtime;
- full fleet management;
- plugin marketplace;
- generic simulator competing with Gazebo;
- custom package registry;
- large driver ecosystem;
- cloud service;
- certification claims.

---

# 11. Recommended immediate next actions

1. Commit this plan as `docs/Neuradix_Implementation_Plan_v0.2.md`.
2. Open RFC-0001 to RFC-0005.
3. Create the initial Rust workspace.
4. Implement the contract parser spike.
5. Implement `DepthMeasurement` end-to-end.
6. Keep v0.4 functional specification as the target reference.
7. Treat Swarm, Aero, Flight and XR as designed-but-deferred expansion tracks.

---

# 12. Final implementation posture

The correct engineering posture is:

> Build the smallest possible Neuradix that proves the core thesis, while designing the contracts and runtime so Swarm, Aero, Flight and XR can be added without re-architecture.

This keeps the platform ambitious without making the first implementation impossible.
