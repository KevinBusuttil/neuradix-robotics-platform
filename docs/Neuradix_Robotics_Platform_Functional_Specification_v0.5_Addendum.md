---
title: "Neuradix Robotics Platform — Embedded and CLI Functional Addendum"
subtitle: "Normative update to Functional Specification v0.4"
author: "Busuttil Technologies Limited"
date: "17 July 2026"
version: "0.5 Addendum Draft"
status: "For review"
applies_to: "Neuradix_Robotics_Platform_Functional_Specification_v0.4.md"
---

# Document status

This document is a normative addendum to the **Neuradix Robotics Platform Product, Functional and Technical Specification v0.4**. It expands the platform ecosystem in two areas that were previously present only at outline level:

1. **Neuradix Embedded**, including Arduino-class boards, ESP32, STM32, RP2040, nRF and other microcontrollers.
2. The **Neuradix CLI command and automation contract**.

Where this addendum conflicts with the corresponding sections of v0.4, this addendum takes precedence. All other v0.4 requirements remain unchanged.

# Platform ecosystem mind map

![Neuradix Robotics Platform ecosystem mind map.](assets/neuradix_platform_ecosystem_mind_map_light.svg)

*Figure A1 — Neuradix Robotics Platform ecosystem, including embedded hardware and the CLI.*

# 1. Updated platform boundary

Neuradix SHALL support one contract and evidence model across multiple execution classes:

```text
Studio / Studio XR / Ground / Fleet
                  |
             Neuradix Edge
                  |
      CAN / serial / RS-485 / Ethernet
                  |
          Neuradix Embedded
        /          |           \
 Embedded Tiny  Embedded MCU  Embedded Connected
   Arduino/AVR  STM32/RP2040     ESP32/nRF/etc.
```

A microcontroller is a first-class Neuradix participant when it:

- implements generated Neuradix contracts;
- has a declared static topology;
- exposes firmware, topology and contract identity;
- reports health and reset state;
- enforces declared local safe-state behaviour;
- communicates through a transport adapter;
- can be represented by the same capability interface in simulation.

A microcontroller is not required to run the full Linux-class runtime, dynamic discovery, Studio, Python, OCI package handling or local MCAP recording.

# 2. Neuradix Embedded profiles

## 2.1 Embedded tiers

| Tier | Typical hardware | Runtime form | Intended functions |
|---|---|---|---|
| **Embedded Tiny** | ATmega/AVR and similarly constrained boards | Generated C/C++ endpoint and static loop | GPIO, ADC, simple sensors, relays, basic actuators |
| **Embedded MCU** | RP2040/RP2350, STM32, nRF52, Renesas RA | Native Rust `no_std` using RTIC, Embassy or static executor | Sensors, motor control, local state machines, deterministic I/O |
| **Embedded Connected** | ESP32-C3/C6/S3, network-capable STM32/nRF | Native Rust or supported C/C++ endpoint with network adapter | Wireless sensor hubs, payload controllers, gateway-capable nodes |
| **Embedded High** | Cortex-M7, higher-end MCU/SoC and RTOS systems | Multiple static components with richer telemetry | Local estimation, advanced motor control, small inference workloads |
| **Edge** | Linux-class SBCs and industrial computers | Full Neuradix runtime | Autonomy, perception, planning, record/replay and supervision |

Board branding SHALL NOT determine the tier. For example, Arduino Uno R3 belongs to Embedded Tiny, while Arduino Uno R4, Nano ESP32 and Portenta-class boards may belong to higher tiers.

## 2.2 Static topology

Embedded deployments MUST use a statically generated topology unless a target-specific profile explicitly permits bounded dynamic behaviour.

```yaml
apiVersion: neuradix.io/v1alpha1
kind: EmbeddedDeployment
metadata:
  name: propulsion-node
spec:
  target: esp32-c3
  executor: embassy
  components:
    - encoder-reader
    - motor-controller
    - thermal-monitor
    - local-safety
  connections:
    - from: encoder-reader.velocity
      to: motor-controller.velocity
    - from: motor-controller.request
      to: local-safety.request
    - from: local-safety.approved
      to: motor-driver.command
```

The embedded graph compiler SHALL generate:

- static dispatch tables;
- bounded queues;
- component identities;
- contract identifiers;
- watchdog configuration;
- memory budgets;
- transport endpoint tables;
- safe-state defaults.

## 2.3 Contract projections

The contract compiler SHALL support at least these projections:

```text
rust-std
rust-no-std
python
protobuf
arduino-cpp
embedded-c
json-schema
```

Generated embedded output SHOULD include:

```text
generated/
├── messages.rs
├── ports.rs
├── topology.rs
├── memory.rs
├── messages.h
├── messages.cpp
├── protocol.c
├── deployment_identity.rs
└── contract_manifest.bin
```

The authored Neuradix contract remains the source of truth. C, C++, Rust and Python projections MUST carry the same schema identity and semantic metadata.

## 2.4 Executors

Neuradix Embedded SHOULD support:

- **Embassy** for asynchronous drivers, connected sensors and low-power services;
- **RTIC** for interrupt-driven deterministic control;
- a minimal static loop profile for highly constrained hardware;
- selected RTOS adapters where their lifecycle and memory behaviour can be bounded.

The public component model MUST remain independent from the selected executor.

## 2.5 Memory and timing

Embedded profiles MUST:

- prohibit unbounded queues;
- declare static RAM and flash budgets;
- expose stack or task memory budgets where supported;
- report deadline misses;
- define queue overflow behaviour;
- avoid heap allocation after initialization where the profile requires it;
- expose build-time resource reports.

## 2.6 Local safety and authority

Safety-relevant embedded actuator nodes MUST enforce local constraints even when Edge, Ground, Fleet, Studio or XR are unavailable.

```text
Edge command request
        |
        v
Embedded authority lease check
        |
        v
Current / temperature / rate / range limits
        |
        v
Actuator output
```

When a command lease expires, communication fails or a watchdog trips, the embedded node MUST enter its declared safe state.

The safe state may include:

- zero output;
- controlled ramp-down;
- hold position;
- mechanical brake;
- surface/land request;
- local emergency sequence;
- hardware interlock activation.

## 2.7 Health and identity

Every embedded node SHALL expose, where supported:

- firmware version;
- source revision;
- deployment manifest hash;
- contract hashes;
- target and board identifier;
- uptime;
- reset reason;
- watchdog state;
- queue overflow count;
- deadline misses;
- communication errors;
- sensor or actuator errors;
- supply voltage;
- temperature;
- current lifecycle and safe-state mode.

## 2.8 Transports

Embedded transports MAY include:

- UART/serial;
- RS-485;
- CAN and CAN FD;
- SPI or I²C for local device links;
- Ethernet;
- UDP/TCP where appropriate;
- Wi-Fi;
- BLE;
- Thread/6LoWPAN;
- Zenoh-Pico as an optional backend;
- mission-specific buses through controlled adapters.

Transport selection MUST NOT alter component-domain logic or contract identity.

## 2.9 Simulation parity

Each hardware capability SHOULD have host simulation and replay implementations.

```rust
pub trait MotorOutput {
    fn set_duty(&mut self, duty: NormalizedDuty) -> Result<()>;
}
```

Possible implementations:

```text
Esp32PwmMotorOutput
Stm32PwmMotorOutput
ArduinoCppMotorOutput
SimulatedMotorOutput
RecordedMotorOutput
```

The same component contracts and safety semantics SHALL be used in simulation and hardware builds.

# 3. Embedded board support policy

## 3.1 Initial target order

The recommended first native targets are:

1. host simulation;
2. ESP32-C3;
3. RP2040;
4. STM32F4 or STM32G4;
5. nRF52;
6. ESP32-S3;
7. Renesas RA4M1 / Arduino Uno R4;
8. Zephyr-supported targets;
9. classic Arduino/AVR through generated C/C++.

## 3.2 Support levels

| Level | Meaning |
|---|---|
| **Experimental** | Builds or examples exist; no compatibility guarantee |
| **Preview** | Automated builds and basic conformance tests |
| **Supported** | Documented toolchain, CI target and release testing |
| **Qualified-by-project** | Mission/project-specific evidence; not a general certification claim |
| **Deprecated** | Supported for migration only |
| **Removed** | No longer built or tested |

# 4. Embedded normative requirements

| ID | Requirement |
|---|---|
| `NRX-EMB-001` | The Embedded profile SHALL support statically generated component topologies. |
| `NRX-EMB-002` | Embedded communication buffers SHALL be bounded at compile time or initialization. |
| `NRX-EMB-003` | The platform SHALL support native `no_std` Rust components on selected MCU families. |
| `NRX-EMB-004` | The platform SHALL support generated C/C++ endpoints for constrained or legacy MCUs. |
| `NRX-EMB-005` | Embedded nodes SHALL expose firmware, topology and contract identities. |
| `NRX-EMB-006` | Safety-relevant nodes SHALL enter a declared safe state when command authority expires. |
| `NRX-EMB-007` | The same capability contracts SHALL be usable by hardware, simulation and replay implementations. |
| `NRX-EMB-008` | Transport selection SHALL NOT change component-domain logic. |
| `NRX-EMB-009` | Production embedded profiles SHALL prohibit unbounded allocation and queues. |
| `NRX-EMB-010` | Embedded nodes SHALL expose health, reset reason and timing/communication failures. |
| `NRX-EMB-011` | The build system SHALL report flash, static RAM and configured stack/task budgets. |
| `NRX-EMB-012` | Supported target families SHALL have automated build and conformance tests. |
| `NRX-EMB-013` | The Embedded public API SHALL be independent of RTIC, Embassy and transport-specific types. |
| `NRX-EMB-014` | Direct actuator access SHALL be limited to declared safety or hardware-capability components. |
| `NRX-EMB-015` | Firmware images SHALL include a content-addressed deployment identity. |
| `NRX-EMB-016` | Embedded update mechanisms SHALL verify target compatibility and signature metadata where supported. |
| `NRX-EMB-017` | Classic Arduino/AVR support MAY use generated C/C++ rather than native Rust. |
| `NRX-EMB-018` | The platform SHALL distinguish development wireless links from safety-relevant command links. |
| `NRX-EMB-019` | Embedded node disconnection SHALL NOT compromise the safety of other nodes. |
| `NRX-EMB-020` | Every embedded actuator deployment SHALL define communication-loss and watchdog responses. |

# 5. Neuradix CLI product contract

## 5.1 Purpose

The canonical CLI executable is:

```bash
neuradix
```

The CLI is a stable automation interface for developers, CI systems, test laboratories, operators and deployment tooling. It SHALL not be treated as a collection of unrelated debugging commands.

## 5.2 Command hierarchy

```text
neuradix
├── new
├── init
├── build
├── check
├── contract
├── component
├── graph
├── run
├── stop
├── inspect
├── record
├── replay
├── explain
├── sim
├── test
├── embedded
├── package
├── deploy
├── registry
├── bridge
├── swarm
├── aero
├── ground
├── fleet
├── studio
├── config
├── context
├── auth
├── doctor
├── completion
└── version
```

Commands MAY be implemented incrementally, but the naming and resource hierarchy SHOULD be stabilized before public preview.

## 5.3 Global options

Applicable commands SHOULD support:

```text
--context <name>
--profile <edge|embedded|sim|ground|flight|safety>
--robot <identity>
--swarm <identity>
--output <table|json|yaml|jsonl>
--offline
--timeout <duration>
--at <timestamp>
--dry-run
--yes
--verbose
--quiet
```

## 5.4 Machine-readable output

Inspection, validation and planning commands MUST support stable machine-readable output.

```bash
neuradix graph --output json
neuradix contract validate contract.yaml --output json
neuradix component health --output jsonl
```

Output schemas SHALL be versioned and documented.

## 5.5 Exit codes

| Code | Meaning |
|---:|---|
| `0` | Success |
| `1` | General operation failure |
| `2` | Invalid CLI use |
| `3` | Contract validation failure |
| `4` | Compatibility failure |
| `5` | Connection/discovery failure |
| `6` | Authentication failure |
| `7` | Authorization denied |
| `8` | Safety rejection |
| `9` | Determinism or replay mismatch |
| `10` | Deployment validation failure |
| `11` | Partial operation |
| `12` | Timeout |

Exit-code meanings SHALL remain backward-compatible within a major CLI version.

## 5.6 Safety rule

The CLI MUST NOT bypass the normal authority path.

```text
CLI semantic intent
      |
      v
Ground identity and authority
      |
      v
Swarm or mission coordination
      |
      v
Onboard Safety
      |
      v
Embedded/Flight controller
```

Development-only direct hardware commands MUST require an explicit hardware-test profile, elevated authority, a reason and audit recording. They MUST be disabled by production deployment policy.

# 6. CLI command groups

## 6.1 Contracts

```bash
neuradix contract validate contracts/
neuradix contract generate --language rust
neuradix contract generate --language python
neuradix contract generate --target rust-no-std --board esp32-c3
neuradix contract generate --target arduino-cpp --board uno-r3
neuradix contract diff Old@1 New@2
neuradix contract compatibility old.yaml new.yaml
neuradix contract inspect VehiclePose
```

## 6.2 Runtime and inspection

```bash
neuradix run deployment.yaml --profile edge
neuradix graph --live
neuradix component list
neuradix component inspect depth-controller
neuradix component health object-detector
neuradix inspect stream navigation/depth
neuradix inspect frames --validate
neuradix inspect clocks --sync-status
neuradix inspect authority thrusters
```

## 6.3 Record, replay and explain

```bash
neuradix record start mission.mcap
neuradix record verify mission.mcap
neuradix replay run mission.mcap --lockstep
neuradix replay branch mission.mcap --replace controller=v2
neuradix explain command thrusters/vertical --at 14:32:18.420
neuradix explain safety-decision saf-9281
neuradix explain task-allocation allocation-42
```

## 6.4 Simulation and testing

```bash
neuradix sim run scenarios/depth-hold.yaml
neuradix sim inject sensor-loss --sensor dvl --at 120s
neuradix sim monte-carlo scenarios/survey.yaml --runs 1000
neuradix test determinism scenarios/depth-hold.yaml
neuradix test replay mission.mcap
neuradix test conformance driver/depth-sensor
```

## 6.5 Embedded

```text
neuradix embedded
├── targets
├── new
├── check
├── generate
├── build
├── flash
├── monitor
├── inspect
├── test
├── size
├── provision
└── update
```

Examples:

```bash
neuradix embedded targets
neuradix embedded new motor-controller --target esp32-c3 --executor embassy
neuradix embedded new temperature-node --target arduino-uno-r3 --language cpp
neuradix embedded generate --manifest embedded/motor-node.yaml
neuradix embedded build --release
neuradix embedded size
neuradix embedded flash --port /dev/ttyUSB0
neuradix embedded monitor
```

## 6.6 Deployment

```bash
neuradix package build
neuradix package sign target/package
neuradix package sbom target/package
neuradix deploy validate deployment.yaml
neuradix deploy plan deployment.yaml
neuradix deploy apply deployment.yaml --dry-run
neuradix deploy rollback robot-01 --to previous
```

## 6.7 Swarm and Aero

```bash
neuradix swarm status survey-alpha
neuradix swarm members survey-alpha
neuradix swarm tasks list survey-alpha
neuradix swarm formation set survey-alpha adaptive-grid
neuradix swarm partitions survey-alpha

neuradix aero airspace validate mission-airspace.yaml
neuradix aero conflict predict --horizon 60s
neuradix aero landing-zones rank --vehicle uav-03
```

# 7. CLI normative requirements

| ID | Requirement |
|---|---|
| `NRX-CLI-001` | The canonical executable SHALL be `neuradix`. |
| `NRX-CLI-002` | The CLI SHALL use a consistent resource-oriented command hierarchy. |
| `NRX-CLI-003` | Validation and inspection commands SHALL support machine-readable output. |
| `NRX-CLI-004` | Machine-readable schemas SHALL be versioned. |
| `NRX-CLI-005` | Exit-code meanings SHALL be documented and stable within a major version. |
| `NRX-CLI-006` | Mutating operational commands SHALL support `--dry-run` where meaningful. |
| `NRX-CLI-007` | Dangerous commands SHALL require explicit confirmation unless a policy-approved automation mode is used. |
| `NRX-CLI-008` | The CLI SHALL NOT bypass Ground, Swarm or onboard Safety authority paths. |
| `NRX-CLI-009` | Development-only direct actuator commands SHALL be disabled in production profiles. |
| `NRX-CLI-010` | CLI actions affecting a live system SHALL be auditable. |
| `NRX-CLI-011` | The CLI SHALL support offline operation for local build, validation, simulation and recording inspection. |
| `NRX-CLI-012` | Shell completion SHOULD be provided for supported shells. |
| `NRX-CLI-013` | Commands SHALL expose timeouts rather than waiting indefinitely. |
| `NRX-CLI-014` | Errors SHALL identify whether failure occurred in parsing, validation, authorization, safety or execution. |
| `NRX-CLI-015` | The CLI SHALL support contexts for selecting local, laboratory, robot, fleet or simulation endpoints. |
| `NRX-CLI-016` | Embedded commands SHALL support target discovery, build, size reporting, flash and monitor workflows. |
| `NRX-CLI-017` | Deployment commands SHALL validate contracts, resources, permissions and safety paths before apply. |
| `NRX-CLI-018` | Deprecations SHALL emit actionable migration guidance. |
| `NRX-CLI-019` | The CLI SHOULD provide structured progress events for automation. |
| `NRX-CLI-020` | `neuradix doctor` SHALL report toolchain, target, connectivity and configuration diagnostics. |

# 8. Repository additions

The repository SHOULD add:

```text
crates/
  embedded-core/
  embedded-codegen/
  embedded-transport/
  cli/

embedded/
  boards/
  examples/
  conformance/

docs/
  Neuradix_CLI_Command_Specification_v0.1.md
  Neuradix_Embedded_Profile_Implementation_Plan_v0.1.md
```

# 9. Updated delivery sequence

The implementation sequence remains conservative:

1. contract parser and code generation;
2. time, frames and deterministic testkit;
3. local data plane and runtime;
4. record/replay, safety and explain;
5. single AUV simulation;
6. **one host-simulated embedded component**;
7. **one native ESP32-C3 or RP2040 component**;
8. **one generated Arduino C++ endpoint**;
9. Studio embedded health inspection;
10. Swarm, XR and Aero expansion;
11. Flight profile later.

The embedded work must prove the shared-contract thesis without delaying the single-AUV vertical slice.

# 10. Updated acceptance criteria

In addition to v0.4 criteria, a platform preview SHOULD demonstrate:

1. one contract generated for Rust `std`, Rust `no_std`, Python and Arduino C++;
2. one MCU node publishing health and sensor data;
3. one actuator node enforcing lease expiry and safe state;
4. host simulation using the same capability contract;
5. CLI build/flash/monitor workflow;
6. build report showing flash and RAM usage;
7. Studio or CLI displaying firmware and contract identity;
8. recorded Edge-side evidence preserving the embedded source identity and timestamps.

# 11. Decisions required before implementation

The project should ratify:

1. the authored contract format as the source of truth;
2. the first native MCU target;
3. the first C/C++ compatibility target;
4. whether Embedded v1 uses Embassy, RTIC or both;
5. the initial serial/CAN framing;
6. the CLI global-option and exit-code contract;
7. the direct-hardware test-mode safety policy;
8. the board-support maturity model;
9. how deployment hashes are encoded in firmware;
10. the minimum embedded conformance suite.
