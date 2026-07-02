---
title: "Neuradix Studio XR — Implementation Plan"
subtitle: "Contract-native observability, replay, 3D visualization and supervised spatial interaction"
author: "Engineering"
date: "02 July 2026"
version: "0.2 Draft"
status: "For review"
supersedes: "Neuradix_Studio_Implementation_Plan_v0.1.md"
related_specification: "Neuradix_Robotics_Platform_Functional_Specification_v0.4.md"
---

# 0. Purpose

This document defines the implementation plan for **Neuradix Studio XR**, the visual engineering and operations interface for the Neuradix Robotics Platform.

It updates the earlier Studio implementation plan to align with Functional Specification v0.4, adding:

- formal Studio XR scope;
- marine AUV swarm visualization;
- UAV swarm visualization;
- 3D headset support;
- semantic spatial commands;
- live/predicted/simulated/replay state separation;
- multi-user operational authority;
- digital-twin command previews;
- Swarm and Aero views;
- safety and command-lineage visualization.

The key product decision remains:

> Studio is a contract-native observability and explanation product first. Commanding and authoring are privileged capabilities added later through Neuradix Ground and Safety.

---

# 1. Product positioning

## 1.1 Why Neuradix Studio exists

Generic robotics visualization tools can render messages. Neuradix Studio must understand what the messages mean.

Studio is differentiated because it can read the same Neuradix contracts, time model, frames, safety authority and recording metadata as the runtime.

It therefore understands:

- units;
- coordinate frames;
- clock domains;
- validity periods;
- uncertainty;
- authority leases;
- safety rejections;
- command lineage;
- component versions;
- schema hashes;
- mission topology;
- live versus replay state;
- simulated versus physical state.

## 1.2 Product variants

Use one shared core engine and multiple shells.

```text
Neuradix Studio
├── Studio Web
│   └── local-first browser/PWA interface
├── Studio Desktop
│   └── Tauri wrapper with native filesystem and GPU access
└── Studio XR
    ├── WebXR headset mode
    └── OpenXR native headset mode
```

These are not separate products. They are shells over a common Rust `studio-engine`.

---

# 2. Non-negotiable principles

## 2.1 Local-first

Studio must open recordings, inspect contracts and render mission data without internet or cloud dependency.

## 2.2 Read-only by default

A freshly installed Studio can inspect but cannot command. Commanding requires explicit connection to Neuradix Ground with authenticated authority.

## 2.3 No authority by visualization

Connecting a headset does not grant actuator authority. Head movements and hand gestures are UI input only.

## 2.4 Semantic commands only

Studio XR should generate operator intents such as:

- inspect this region;
- avoid this volume;
- return this vehicle;
- split the swarm;
- hold formation;
- emergency land;
- surface AUV-3.

It must not directly produce motor, thruster or servo commands.

## 2.5 Distinguish state truth categories

Studio must visually distinguish:

- live measured;
- estimated;
- predicted;
- stale;
- simulated;
- replayed.

This is essential for XR, AUV swarms and UAV swarms.

## 2.6 One engine

Browser, desktop and XR should share the same Rust engine for:

- MCAP indexing;
- contract handling;
- time model;
- frame transforms;
- plot data preparation;
- 3D scene data;
- replay;
- command lineage;
- safety overlays.

---

# 3. Architecture

```text
┌────────────────────────────────────────────────────────────────────┐
│ Shells                                                             │
│  • Browser/PWA             • Tauri Desktop                         │
│  • WebXR Headset           • OpenXR Native Headset                 │
└───────────────┬────────────────────────────────────────────────────┘
                │
┌───────────────▼────────────────────────────────────────────────────┐
│ Web/UI Layer                                                        │
│  • TypeScript + React                                               │
│  • Dockable panels                                                  │
│  • Command palette                                                  │
│  • Timeline controls                                                │
│  • XR spatial UI layer                                               │
└───────────────┬────────────────────────────────────────────────────┘
                │ typed WASM/native interface
┌───────────────▼────────────────────────────────────────────────────┐
│ studio-engine                                                       │
│  • MCAP read/index/decimate                                         │
│  • contract/schema awareness                                        │
│  • time and clock-domain handling                                   │
│  • frame/transform resolution                                       │
│  • lineage/explain queries                                          │
│  • safety/authority overlays                                        │
│  • health model                                                     │
│  • swarm state model                                                │
│  • airspace and marine world models                                 │
│  • wgpu renderer                                                    │
│  • live data client                                                 │
└───────────────┬──────────────────────────────┬─────────────────────┘
                │                              │
┌───────────────▼─────────────┐    ┌───────────▼─────────────────────┐
│ Recordings                   │    │ studio-backend                   │
│  • MCAP                      │    │  • live Neuradix graph bridge    │
│  • manifests                 │    │  • WebSocket                     │
│  • schemas                   │    │  • Foxglove-compatible endpoint  │
│  • hashes                    │    │  • Neuradix-native endpoint      │
└─────────────────────────────┘    └─────────────────────────────────┘
```

---

# 4. Technology choices

| Concern | Choice | Rationale |
|---|---|---|
| Core engine | Rust | Same semantics as runtime, performance, safety |
| Browser integration | WASM via wasm-bindgen | Shared engine in browser |
| Desktop shell | Tauri | Rust-native, lighter than Electron |
| Web UI | TypeScript + React | Mature ecosystem, fast development |
| Renderer | wgpu | Native + WebGPU path |
| Recording | MCAP via Neuradix Record crates | Avoid duplicate log reader |
| Live transport | WebSocket initially | Browser-friendly |
| Interop | Foxglove WebSocket compatibility | Migration path |
| XR browser | WebXR | Browser headset path |
| XR native | OpenXR | Native headset path |
| Plotting | GPU-backed where possible | Large logs and high-rate data |
| Data model | Neuradix contracts | Semantic visualization |

---

# 5. Studio panels

## 5.1 Core inspection panels

| Panel | Purpose |
|---|---|
| Graph Panel | Component graph, streams, commands and dependencies |
| Contract Panel | Contract metadata, schema hash, units, frames, clock domain |
| Timeline Panel | Multi-clock timeline, events, validity intervals |
| Plot Panel | Scalar and vector signals with units |
| Health Panel | Component/system health with reasons |
| Record Panel | MCAP metadata, manifest, schema versions, SBOM |
| Explain Panel | Causal chain from sensor to command |
| Safety Panel | Authority leases, safety constraints, rejections |
| Frame Panel | Transform tree, uncertainty, stale transforms |
| Log/Event Panel | Structured events and diagnostics |

## 5.2 Simulation panels

| Panel | Purpose |
|---|---|
| Scenario Panel | Active scenario, seed, fault injections |
| Replay Panel | Pause, step, branch and compare |
| Digital Twin Panel | Predicted state and simulated counterpart |
| Fault Injection Panel | Inject sensor, link, actuator and environment faults |

## 5.3 Swarm panels

| Panel | Purpose |
|---|---|
| Swarm Membership Panel | Members, roles, epochs, health |
| Task Allocation Panel | Active tasks, assignments, proposals, conflicts |
| Formation Panel | Formation geometry, errors, separations |
| Shared World Model Panel | Federated map items, provenance, confidence |
| Partition Panel | Network partitions, rejoin reconciliation |
| Cooperative Localization Panel | Intervehicle ranges, covariance, map constraints |
| Communication Panel | Link status, latency, bandwidth, priority queues |

## 5.4 Aero panels

| Panel | Purpose |
|---|---|
| Airspace Panel | 3D geofences, restricted volumes, corridors |
| UAV Trajectory Panel | Actual, predicted and planned trajectories |
| Collision Panel | Conflicts, closest approach, avoidance actions |
| Landing/Divert Panel | Emergency landing zones and recovery paths |
| Weather/Wind Panel | Wind estimate and environmental effects |
| Airframe Panel | Airframe constraints: hover, turn radius, stall speed |

## 5.5 Marine panels

| Panel | Purpose |
|---|---|
| Bathymetry Panel | Seabed map and terrain |
| Acoustic Link Panel | Underwater communication links and delay |
| Sonar Coverage Panel | Sonar cones, coverage, detections |
| AUV Uncertainty Panel | Pose uncertainty and stale positions |
| Mission Survey Panel | Completed, pending and uncertain coverage |
| Surface Gateway Panel | Relay state and data upload status |

---

# 6. XR functional design

## 6.1 XR scene model

Studio XR should render a spatial world containing:

- robots;
- planned trajectories;
- actual trails;
- predicted future states;
- uncertainty volumes;
- sensor coverage;
- communication links;
- mission regions;
- geofences or exclusion zones;
- targets and detections;
- environmental models;
- safety events.

## 6.2 XR interaction modes

| Mode | Authority | Purpose |
|---|---|---|
| Observe | None | Inspect live, simulated or replayed mission |
| Supervise | Limited | Approve tasks, pause/resume mission, request status |
| Mission Intervention | Elevated | Define region, reassign vehicles, change formation |
| Emergency | Emergency authority | Surface, land, abort, hold, return |
| Flight-test Direct | Special test authority only | Limited direct piloting under controlled conditions |

The default mode is **Observe**.

## 6.3 Operator intent flow

```text
head/hand/controller input
        │
        ▼
XR spatial interaction
        │
        ▼
semantic operator intent
        │
        ▼
preview and validation
        │
        ▼
Ground authority check
        │
        ▼
Swarm or vehicle command
        │
        ▼
onboard Safety validation
        │
        ▼
execution or rejection
```

## 6.4 Example operator intents

```yaml
operator_intent:
  type: inspect_region
  region:
    frame: earth/enu
    polygon: [[0,0], [50,0], [50,30], [0,30]]
  preferences:
    minimum_vehicles: 2
    sensor: optical_camera
  authority:
    role_required: mission_supervisor
    expires_after: 30s
```

```yaml
operator_intent:
  type: emergency_land
  vehicle: uav-03
  reason: low_battery_and_wind_risk
  authority:
    role_required: safety_officer
```

```yaml
operator_intent:
  type: surface_vehicle
  vehicle: auv-02
  reason: localization_uncertainty_exceeded
```

---

# 7. Visualization truth model

Every rendered object must carry a truth category.

| Category | Meaning | Example rendering |
|---|---|---|
| Live measured | Recent authoritative telemetry | Solid vehicle |
| Estimated | Current estimate from old data/model | Solid with uncertainty halo |
| Predicted | Future state from model | Dashed path or ghost vehicle |
| Stale | State older than validity interval | Faded or warning outline |
| Simulated | Digital twin, not physical telemetry | Blue/transparent simulation style |
| Replayed | Historical recording | Timeline/replay watermark |

Studio must not render predicted or stale data as if it were live truth.

---

# 8. Command preview

Before committing significant commands, Studio XR should use digital twins to preview:

- path feasibility;
- collision risk;
- energy cost;
- communication impact;
- safety constraints;
- timing;
- task completion estimate;
- alternative plans.

The preview result should include:

```yaml
preview_result:
  status: warning
  predicted_completion_time: 38min
  energy_remaining_minimum: 24%
  risks:
    - vehicle: uav-02
      issue: possible_collision_conflict
      time_to_conflict: 18s
  recommendations:
    - increase_separation_to: 18m
```

---

# 9. Swarm visualization implementation

## 9.1 Data model

Studio engine needs a `SwarmViewModel`:

```text
SwarmViewModel
├── SwarmIdentity
├── MembershipEpoch
├── Members[]
├── Roles[]
├── Tasks[]
├── TaskAllocations[]
├── Formations[]
├── Partitions[]
├── SharedWorldItems[]
├── CommunicationLinks[]
├── SafetyConstraints[]
└── TimelineEvents[]
```

## 9.2 Visual overlays

- current roles;
- membership epoch;
- vehicle health;
- task ownership;
- unassigned tasks;
- formation error;
- communication topology;
- partition warning;
- rejoin reconciliation;
- world-model conflicts;
- local safety overrides.

## 9.3 First Swarm Studio demo

A recorded three-AUV mission:

1. Open mission recording.
2. Show three vehicles and their tasks.
3. Show acoustic link degradation.
4. Show partition.
5. Show reallocation.
6. Show rejoin.
7. Explain why one AUV left the formation.

---

# 10. Aero visualization implementation

## 10.1 Data model

```text
AeroViewModel
├── AirspaceVolumes[]
├── UAVStates[]
├── PlannedTrajectories[]
├── PredictedTrajectories[]
├── CollisionConflicts[]
├── LandingZones[]
├── WeatherWind[]
├── CommunicationLinks[]
├── SensorCoverage[]
└── AuthorityState
```

## 10.2 Required overlays

- 3D airspace volumes;
- UAV actual paths;
- UAV predicted paths;
- collision conflict cones;
- minimum separation;
- geofence violation risk;
- emergency landing zones;
- wind vectors;
- video/camera cones;
- flight mode;
- stale telemetry indicator.

## 10.3 First Aero Studio demo

A simulated four-UAV mission:

1. Open live simulation.
2. Show airspace volume.
3. Operator draws inspection volume.
4. Studio previews assignment.
5. Collision risk appears.
6. Digital twin suggests increased separation.
7. Operator approves.
8. Mission executes and is recorded.

---

# 11. Marine visualization implementation

## 11.1 Data model

```text
MarineViewModel
├── Bathymetry
├── AUVStates[]
├── AcousticLinks[]
├── SonarCoverage[]
├── SurveyCoverage
├── Detections[]
├── PoseUncertainty[]
├── SurfaceGateways[]
├── PredictedSurfacingPaths[]
└── MissionEvents[]
```

## 11.2 Required overlays

- seabed mesh;
- AUV pose uncertainty;
- stale position markers;
- sonar field of view;
- acoustic connectivity;
- survey coverage;
- mission sectors;
- surface gateway;
- detected anomaly;
- predicted surfacing path.

## 11.3 First Marine XR demo

A headset replay of AUV swarm survey:

1. Display seabed and mission area.
2. Show three AUVs with uncertainty volumes.
3. Show acoustic links.
4. Operator selects anomaly.
5. Studio creates inspect-region intent.
6. Swarm preview assigns two vehicles.
7. Operator approves in simulation.
8. Safety rejects one vehicle due to energy reserve.
9. Studio explains the rejection.

---

# 12. Implementation phases

## Phase S0 — Studio engine nucleus

**Duration:** 6–8 weeks after core contracts/record crates exist.

### Deliverables

- `studio-engine` crate.
- Read MCAP file.
- Read contract metadata.
- Timeline index.
- Scalar plot data preparation.
- Component graph model.
- Basic web shell.

### Exit criteria

- Open a recorded mission.
- Display graph, timeline and scalar stream.

---

## Phase S1 — Neuradix-native inspection

**Duration:** 8–10 weeks.

### Deliverables

- contract panel;
- time/clock-domain display;
- frame panel;
- health panel;
- safety panel;
- explain panel;
- stale data display.

### Exit criteria

- Click actuator command and see lineage.
- Stale data visibly marked.
- Safety rejection visibly explained.

---

## Phase S2 — 3D scene and simulation view

**Duration:** 10–14 weeks.

### Deliverables

- wgpu 3D renderer;
- vehicle models;
- path trails;
- sensor cones;
- uncertainty volumes;
- digital twin display;
- live simulation connection.

### Exit criteria

- Replay AUV depth-hold mission in 3D.
- Display predicted versus measured state distinctly.

---

## Phase S3 — Swarm and marine XR

**Duration:** 12–16 weeks.

### Deliverables

- Swarm view model;
- membership/role panels;
- formation and task overlays;
- AUV acoustic links;
- bathymetry/sonar overlays;
- headset observe mode.

### Exit criteria

- Replay three-AUV mission in headset.
- Display partition/rejoin and task reallocation.

---

## Phase S4 — Spatial operator intents

**Duration:** 12–16 weeks.

### Deliverables

- spatial region selection;
- operator intent builder;
- command preview;
- Ground authority integration;
- Safety result display;
- audit trail.

### Exit criteria

- Operator defines an inspection region in XR.
- System previews assignment.
- Command is approved via Ground and recorded.

---

## Phase S5 — Aero XR

**Duration:** after Aero simulation exists.

### Deliverables

- airspace 3D volumes;
- UAV predicted trajectories;
- collision overlays;
- geofence overlays;
- emergency landing zones;
- wind/weather overlays;
- assisted-piloting UI prototype.

### Exit criteria

- Four-UAV simulated mission visualized in XR.
- Collision avoidance override is visible and explainable.

---

## Phase S6 — Multi-user operations

**Duration:** later, after Ground authority is stable.

### Deliverables

- multiple headset/browser sessions;
- operator roles;
- command ownership;
- approval workflow;
- safety officer override;
- collaborative annotations.

### Exit criteria

- Mission commander proposes task.
- Safety officer approves emergency action.
- Observer remains read-only.

---

# 13. Performance requirements

| Scenario | Target |
|---|---|
| Open multi-GB MCAP | First useful frame within seconds using lazy indexing |
| Timeline scrub | Interactive and bounded memory |
| Scalar plots | GPU/LOD decimation for high-rate signals |
| Point cloud/sonar | GPU buffers and culling |
| Live stream | Backpressure; Studio must not stall robot |
| XR rendering | Stable headset frame rate for simplified scene |
| Large swarm | Graceful degradation through aggregation and LOD |
| Replay | Step, pause, accelerate and branch |

---

# 14. Safety and authority requirements

| ID | Requirement |
|---|---|
| STU-XR-001 | Studio SHALL be read-only by default. |
| STU-XR-002 | Headset connection SHALL NOT grant command authority. |
| STU-XR-003 | XR interactions SHALL generate semantic operator intents, not actuator commands. |
| STU-XR-004 | Commands SHALL pass through Neuradix Ground authority. |
| STU-XR-005 | Onboard Safety SHALL remain final authority for vehicle execution. |
| STU-XR-006 | Studio SHALL clearly show live, predicted, simulated, stale and replayed state. |
| STU-XR-007 | Command previews SHALL show risks and safety warnings. |
| STU-XR-008 | Multi-user sessions SHALL identify command proposer, approver and executor. |
| STU-XR-009 | Emergency actions SHALL be audited and recorded. |
| STU-XR-010 | Studio disconnection SHALL NOT affect robot autonomy or safety. |

---

# 15. First 90-day Studio plan

This starts only after the core record/contract crates are minimally usable.

## Weeks 1–2

- Create `studio-engine`.
- Read simple MCAP.
- Read embedded contract metadata.
- Build web shell.

## Weeks 3–4

- Timeline index.
- Scalar plot.
- Contract panel.
- Component graph panel.

## Weeks 5–6

- Health panel.
- Safety decision panel.
- Basic explain panel.
- Stale data markers.

## Weeks 7–8

- Basic 3D scene.
- Vehicle marker.
- Path trail.
- replay clock control.

## Weeks 9–10

- Digital twin overlay.
- Predicted versus measured visual distinction.
- frame transform display.

## Weeks 11–12

- Package as local PWA/Tauri prototype.
- Demonstrate AUV depth-hold recording and command explanation.

---

# 16. What not to build first

Do not start with:

- headset-first UI;
- multi-user collaboration;
- rich mission authoring;
- production command interface;
- full 3D GIS;
- full UAV traffic management;
- complete Foxglove replacement;
- cloud backend;
- marketplace;
- complex avatar/social XR features.

Start with inspection, replay and explanation.

---

# 17. Version 1.0 Studio acceptance criteria

Studio v1.0 alpha is acceptable when it can:

1. Open a Neuradix MCAP recording.
2. Display component graph.
3. Display contract metadata.
4. Display scalar stream with units.
5. Display time domains and stale data.
6. Display health state.
7. Display safety rejection.
8. Explain one actuator command.
9. Replay mission deterministically.
10. Render a basic 3D mission scene.
11. Connect to live local simulation.
12. Avoid affecting robot runtime when disconnected.
13. Run locally without cloud.
14. Package as browser/PWA and desktop prototype.

---

# 18. Final Studio posture

Neuradix Studio XR should become one of the platform's strongest differentiators, but it must grow from the core engineering facts:

```text
contracts → records → replay → explain → 3D view → XR interaction → supervised command
```

Do not invert that order. A beautiful headset interface that cannot explain a command or distinguish stale from live state would undermine the platform's central promise.

The correct product posture is:

> Neuradix Studio XR lets humans understand, supervise and safely influence autonomous robotic systems without becoming an unsafe real-time control path.
