# Neuradix RFC Backlog v0.3

This backlog aligns the architecture RFC sequence with the current functional and implementation documentation.

## P0 — Core foundation

| RFC | Title | Required before |
|---|---|---|
| RFC-0001 | Component and Lifecycle Model | Runtime implementation |
| RFC-0002 | Contract Format and Code Generation | Public SDK and code generation |
| RFC-0003 | Time, Clocks and Deterministic Replay | Runtime and simulation |
| RFC-0004 | Transport-Neutral Data Plane | Transport implementations |
| RFC-0005 | Safety Authority and Command Lineage | Actuator path |
| RFC-0013 | CLI Command, Output and Automation Contract | Public CLI implementation |
| RFC-0014 | Embedded Runtime, Board Support and Code Generation | MCU implementation |
| RFC-0015 | Recording and Deterministic Replay | Record/replay and external log backends (MCAP) |
| RFC-0016 | Deterministic Execution | Executors, scheduling and replay-equivalent runs |
| RFC-0017 | FDIR and Fault Modes | Fault detection/confirmation/recovery and safe modes |
| RFC-0018 | Python Worker SDK and Process Isolation | Isolated Python components and crash safety |
| RFC-0019 | Deployment Graph Validation | Compiling/launching deployments; contracts-before-connectivity gate |
| RFC-0020 | Deterministic Vehicle Simulation | Closed-loop AUV vertical slice; control/safety driving a simulated plant |
| RFC-0021 | MCAP Recording Backend | External tooling interop (Foxglove/ROS 2); cross-container replay equivalence |

## P1 — Multi-robot and domain expansion

| RFC | Title |
|---|---|
| RFC-0006 | Swarm Membership and Task Allocation |
| RFC-0007 | Federated Shared World Model |
| RFC-0008 | Studio XR Operator Intent and Authority |
| RFC-0009 | Aero Airspace and Collision Safety Model |
| RFC-0010 | Multi-Vehicle Simulation and Replay |

## P2 — Operations and flight

| RFC | Title |
|---|---|
| RFC-0011 | Flight Profile and Restricted Rust Policy |
| RFC-0012 | Ground/Fleet Identity and Deployment Manifest |

## RFC-0013 must decide

- canonical command hierarchy;
- global flags;
- contexts;
- JSON/YAML/JSONL result schemas;
- stable exit codes;
- error taxonomy;
- confirmations and `--dry-run`;
- authentication and authority;
- safety restrictions;
- deprecation policy;
- shell completion.

## RFC-0014 must decide

- Embedded Tiny/MCU/Connected/High boundaries;
- first native MCU target;
- first Arduino/C++ target;
- `no_std` component API;
- Embassy and RTIC adapter boundaries;
- static topology generation;
- memory and timing budgets;
- serial/CAN framing;
- deployment identity in firmware;
- health and watchdog contracts;
- local safe-state rules;
- target support levels and conformance tests.
