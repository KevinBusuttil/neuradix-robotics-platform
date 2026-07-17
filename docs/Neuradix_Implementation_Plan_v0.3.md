---
title: "Neuradix Robotics Platform — Implementation Plan"
subtitle: "Engineering execution plan aligned to Functional Specification v0.4 and Embedded/CLI Addendum v0.5"
author: "Engineering"
date: "17 July 2026"
version: "0.3 Draft"
status: "For review"
supersedes: "Neuradix_Implementation_Plan_v0.2.md"
---

# 0. Purpose

This document updates the Neuradix implementation plan to include:

- the full CLI command and automation contract;
- Neuradix Embedded tiers;
- Arduino/AVR generated C/C++ endpoints;
- native Rust support for ESP32, RP2040, STM32 and selected nRF/MCU targets;
- embedded safety, health, identity and deployment;
- Studio inspection of embedded nodes;
- the light-theme ecosystem mind map.

![Neuradix Robotics Platform ecosystem mind map.](assets/neuradix_platform_ecosystem_mind_map_light.svg)

The core sequencing rule remains unchanged:

> Build one complete vertical slice before widening the platform.

The first complete slice remains a simulated AUV depth-control mission. Embedded work is introduced as a narrow extension of that slice, not as a separate ecosystem effort.

# 1. Programme structure

## 1.1 Foundation tracks

| Track | Scope |
|---|---|
| **C — Contracts** | IDL, semantic metadata, schema identity and code generation |
| **T — Time and Frames** | clocks, replay time, units, transforms and uncertainty |
| **D — Data Plane** | Stream/State/Command/Task/Event/Query and transport abstraction |
| **R — Runtime** | lifecycle, execution classes, supervision and health |
| **S — Safety** | authority leases, constraints, local safe states and lineage |
| **REC — Record/Replay** | MCAP, deterministic replay, branch replay and explain |
| **CLI — Command Interface** | stable command tree, JSON output, exit codes and automation |
| **TEST — Verification** | testkit, conformance, fuzzing, timing and replay equivalence |

## 1.2 Deployment and domain tracks

| Track | Scope |
|---|---|
| **EMB — Embedded** | `no_std` Rust, generated C/C++, boards, flashing and conformance |
| **SIM — Simulation** | digital twins, scenarios, faults and HIL |
| **PY — Python** | isolated Python components and AI workflows |
| **STD — Studio/XR** | inspection, 3D, XR and supervised intent |
| **SWM — Swarm** | membership, allocation, world model and partition recovery |
| **AER — Aero** | UAV airspace, collision safety and flight-domain semantics |
| **GND/FLEET** | mission control, authority, assets, deployment and updates |
| **FLT — Flight** | static qualification-oriented runtime and FDIR |
| **BR — Bridges** | ROS 2, MAVLink, CAN, serial, Zenoh and external systems |

# 2. Repository architecture

```text
neuradix-robotics-platform/
├── crates/
│   ├── contracts/
│   ├── time/
│   ├── frames/
│   ├── transport-api/
│   ├── transport-local/
│   ├── transport-shm/
│   ├── transport-zenoh/
│   ├── data-plane/
│   ├── runtime/
│   ├── safety/
│   ├── record/
│   ├── sim/
│   ├── cli/
│   ├── testkit/
│   ├── embedded-core/
│   ├── embedded-codegen/
│   └── embedded-transport/
├── embedded/
│   ├── boards/
│   │   ├── esp32-c3/
│   │   ├── rp2040/
│   │   ├── stm32/
│   │   └── arduino-avr/
│   ├── examples/
│   └── conformance/
├── python/
├── studio/
├── contracts/
├── examples/
├── docs/
└── rfcs/
```

Dependency rules:

```text
contracts → no runtime dependency
time/frames → contracts
transport-api → contracts/time
data-plane → transport-api/contracts/time/frames
runtime → data-plane/contracts/time
safety → runtime/contracts/time/frames
record → contracts/time/frames/data-plane
embedded-core → contracts/time, no full runtime dependency
embedded-codegen → contracts
cli → contracts/runtime/record/sim/embedded tooling
studio-engine → contracts/time/frames/record
```

# 3. CLI implementation track

## 3.1 CLI architecture

Use Rust and a command framework such as `clap`, but keep command semantics independent from the parsing library.

```text
CLI parser
   |
Command request model
   |
Application services
   |
Local runtime / remote Ground / deployment service
   |
Structured result and exit-code mapper
```

The command implementation MUST NOT mix terminal rendering with operation logic. Every operation should produce a structured result that can be rendered as table, JSON, YAML or JSONL.

## 3.2 Initial command set

```bash
neuradix init
neuradix contract validate
neuradix contract generate
neuradix build
neuradix run
neuradix graph
neuradix component list
neuradix component health
neuradix inspect stream
neuradix record start
neuradix record stop
neuradix replay run
neuradix explain command
neuradix sim run
neuradix test determinism
neuradix doctor
```

## 3.3 CLI phases

### CLI-0 — command contract

- ratify command hierarchy;
- ratify global flags;
- ratify output envelopes;
- ratify exit codes;
- define contexts;
- define error taxonomy;
- create shell completion framework.

### CLI-1 — local development

- project initialization;
- contract validation/generation;
- build/run;
- component and stream inspection;
- simulation;
- record/replay;
- explain;
- doctor.

### CLI-2 — packaging and deployment

- package build/sign/verify;
- deployment validate/plan/apply;
- structured progress;
- dry-run;
- rollback;
- registry operations.

### CLI-3 — embedded

- target list;
- project generation;
- build;
- flash;
- monitor;
- size;
- provision;
- conformance test.

### CLI-4 — operations

- Ground authority;
- Fleet;
- Swarm;
- Aero;
- Studio launch/connect;
- audited live mutations.

# 4. Embedded implementation track

## 4.1 Scope boundaries

Embedded v1 SHALL prove:

- shared contracts;
- static topology;
- bounded memory;
- health and identity;
- local safe state;
- simulation parity;
- transport independence;
- CLI toolchain integration.

Embedded v1 SHALL NOT attempt:

- every Arduino board;
- full dynamic discovery;
- Python on MCUs;
- MCU-local MCAP;
- production OTA across every target;
- safety certification;
- arbitrary third-party package loading.

## 4.2 First targets

| Order | Target | Reason |
|---:|---|---|
| 1 | Host simulation | Fast contract/runtime development |
| 2 | ESP32-C3 | Low-cost RISC-V, connectivity, useful robotics node |
| 3 | RP2040 | Simple dual-core Cortex-M ecosystem |
| 4 | STM32F4/G4 | Motor-control and industrial relevance |
| 5 | Arduino Uno R3 C++ projection | Demonstrates legacy compatibility |
| 6 | nRF52 | Low-power wireless |
| 7 | ESP32-S3 / Uno R4 | Broader board ecosystem |

## 4.3 Embedded crates

### `embedded-core`

- static component trait;
- bounded ports;
- health state;
- command lease;
- watchdog;
- deployment identity;
- safe-state interface;
- executor-neutral interfaces.

### `embedded-codegen`

- Rust `no_std` generation;
- Arduino C++ generation;
- embedded C generation;
- topology generation;
- memory report generation;
- target manifests.

### `embedded-transport`

- serial framing;
- CAN framing;
- CRC/sequence handling;
- duplicate suppression;
- timestamp preservation;
- optional Zenoh-Pico gateway adapter.

## 4.4 Executor adapters

Implement in this order:

1. static-loop host simulator;
2. Embassy adapter;
3. RTIC adapter;
4. selected RTOS adapter only after use-case demand.

## 4.5 First embedded reference node

Use the AUV vertical slice:

```text
Edge depth controller
       |
       v
Embedded propulsion node
  - receives thrust request
  - validates lease
  - applies slew/current/thermal limit
  - reports applied output and health
  - enters zero/ramp-down safe state on link loss
```

The same propulsion capability runs as:

- host simulation;
- ESP32-C3 firmware;
- generated Arduino C++ demonstration endpoint.

# 5. Updated phased roadmap

## Phase 0 — RFC and workspace foundation, 4–6 weeks

Deliver:

- RFC-0001 Component Model;
- RFC-0002 Contract Format;
- RFC-0003 Time and Replay;
- RFC-0004 Data Plane;
- RFC-0005 Safety Authority;
- RFC-0013 CLI Contract;
- RFC-0014 Embedded Runtime and Board Support;
- workspace and CI;
- ecosystem mind map in docs.

## Phase 1 — contract-to-runtime path, 8–12 weeks

Deliver:

- contract parser;
- Rust codegen;
- Python stubs;
- in-process stream;
- lifecycle skeleton;
- injected clock;
- initial CLI.

Demonstrate `DepthMeasurement` end-to-end.

## Phase 2 — record/replay/safety nucleus, 10–14 weeks

Deliver:

- MCAP;
- replay clock;
- safety gate;
- lineage;
- Python isolation;
- Studio inspection;
- structured CLI output.

## Phase 3 — single AUV vertical slice, 12–20 weeks

Deliver:

- minimal AUV simulation;
- depth control;
- thruster request;
- safety rejection;
- recording;
- deterministic replay;
- explain command.

This remains the v1.0 alpha target.

## Phase 3E — embedded extension, 8–12 weeks

Start after Phase 3 interfaces stabilize.

Deliver:

- `embedded-core`;
- host simulated propulsion node;
- ESP32-C3 or RP2040 build;
- serial or CAN transport;
- health/identity;
- lease expiry safe state;
- `neuradix embedded` CLI;
- Arduino C++ code generation proof.

## Phase 4 — Swarm Alpha

Three AUVs, partition/rejoin, task reallocation and shared world model.

## Phase 5 — Studio XR Alpha

3D scene, measured/predicted/replay state, spatial intent and command preview.

## Phase 6 — Aero Alpha

UAV simulation, airspace, local collision avoidance and XR overlays.

## Phase 7 — Ground/Fleet

Identity, audit, inventory, deployment and updates.

## Phase 8 — Flight Alpha

Static topology, restricted Rust, scheduler, watchdog, FDIR and evidence prototype.

# 6. First 90 days

## Weeks 1–2

- workspace, CI and governance;
- RFC-0001 through RFC-0005;
- RFC-0013 CLI;
- RFC-0014 Embedded;
- create crates: contracts, time, transport-api, runtime, cli, testkit.

## Weeks 3–4

- contract parser spike;
- `DepthMeasurement`;
- schema normalization/hash;
- `Clock` trait;
- CLI output envelope and exit-code types.

## Weeks 5–6

- in-process stream;
- lifecycle skeleton;
- `neuradix run`;
- `neuradix graph`;
- `neuradix contract validate`;
- first structured JSON output.

## Weeks 7–8

- generated Rust;
- generated Python stubs;
- health state;
- bounded queues;
- replay clock;
- host-only `embedded-core` spike.

## Weeks 9–10

- MCAP write/read;
- safety decision;
- authority lease;
- CLI record/replay stubs;
- embedded deployment identity model.

## Weeks 11–12

- depth-hold simulation;
- safety rejection;
- command explanation;
- preliminary `neuradix embedded targets`;
- architecture review before MCU implementation.

# 7. Embedded conformance suite

Every supported target SHOULD run:

- contract encode/decode vectors;
- timestamp and sequence handling;
- queue overflow tests;
- watchdog reset test;
- command lease expiry;
- safe-state transition;
- health report;
- firmware/deployment identity;
- transport corruption detection;
- resource budget report;
- host-simulation equivalence for selected logic.

# 8. Studio integration

Studio shall later show:

- embedded node inventory;
- firmware and deployment hash;
- reset reason;
- watchdog state;
- supply voltage and temperature;
- deadline misses;
- queue overflows;
- communication quality;
- authority lease status;
- local safety intervention;
- flash/RAM budget from package metadata.

Studio MUST NOT provide a direct unaudited actuator bypass.

# 9. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Embedded scope delays core runtime | High | Phase 3E gate after core interfaces stabilize |
| Too many boards | High | Formal support tiers and first-target list |
| Arduino compatibility forces lowest-common-denominator API | High | Generated endpoint projection, not common runtime implementation |
| Unsafe cross-language/generated code | High | golden vectors, fuzzing and conformance tests |
| CLI becomes unstable scripting surface | High | RFC, structured output and stable exit codes |
| Flash/RAM budgets ignored | High | build reports and CI thresholds |
| Wireless link treated as safety channel | High | explicit link classification and local safe state |
| Executor types leak into SDK | Medium | executor-neutral `embedded-core` |
| OTA added too early | Medium | defer production OTA; start with signed wired flashing |
| Direct debug command bypasses Safety | High | hardware-test profile and audited authority only |

# 10. Updated preview acceptance criteria

A preview is credible when it demonstrates:

1. authored contract to Rust, Python, `no_std` Rust and Arduino C++;
2. single AUV simulation and deterministic replay;
3. command explainability and safety rejection;
4. Python crash isolation;
5. host-simulated embedded propulsion node;
6. one physical MCU node;
7. lease expiry safe state;
8. CLI build/flash/monitor;
9. flash/RAM report;
10. Studio/CLI display of firmware and contract identity;
11. ROS 2 or MAVLink bridge boundary;
12. reproducible public instructions.

# 11. Immediate next actions

1. Approve RFC-0013 and RFC-0014.
2. Choose ESP32-C3 or RP2040 as the first target.
3. Choose serial or CAN as the first embedded transport.
4. Define the CLI JSON result envelope.
5. Implement `DepthMeasurement` through Rust/Python/embedded projections.
6. Preserve the Phase 3 single-AUV gate before broad embedded work.
