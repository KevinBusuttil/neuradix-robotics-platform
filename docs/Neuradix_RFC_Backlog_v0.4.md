# Neuradix RFC Backlog v0.4

**8 September 2026 — proposed decision backlog.** Supersedes v0.3. Aligned with [Neuradix_Robotics_Platform_Functional_Specification_v0.6.md](Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) and [Neuradix_Implementation_Plan_v0.4.md](Neuradix_Implementation_Plan_v0.4.md). Existing RFCs describe their recorded design/implementation snapshot; new requirements need amendments and evidence. This backlog does not ratify decisions or imply implementation.

## Existing core RFCs: retain IDs and amend deliberately

| RFC | Current subject | Required alignment / package |
|---|---|---|
| [0001](rfcs/RFC-0001-Component-and-Lifecycle-Model.md) | Component/lifecycle | Shared profile capabilities and executable graph, B01/B07 |
| [0002](rfcs/RFC-0002-Contract-Format-and-Code-Generation.md) | Contract/codegen | Semantic versus wire identity and ABI compatibility, A02/A03 |
| [0003](rfcs/RFC-0003-Time-Clocks-and-Deterministic-Replay.md) | Time/clocks/replay | Rollover, reboot, uncertainty and replay levels, B03/A07 |
| [0004](rfcs/RFC-0004-Transport-Neutral-Data-Plane.md) | Transport boundaries | Bounded serial/network/discovery and data locality, B06/D03 |
| [0005](rfcs/RFC-0005-Safety-Authority-and-Command-Lineage.md) | Authority/lineage | Trusted evaluation time, finite limits and actual enforcement, A04 |
| [0013](rfcs/RFC-0013-CLI-Command-Output-and-Automation-Contract.md) | CLI/output | v0.2 project/job families and digest/program-replay migration, A07/C04/D01 |
| [0014](rfcs/RFC-0014-Embedded-Runtime-and-Board-Support.md) | Embedded/boards | Paired actual Uno/MCU gate, ABI and board evidence, B04/B05 |
| [0015](rfcs/RFC-0015-Recording-and-Deterministic-Replay.md) | Recording/replay | Maintained MCAP, independent fixtures and bounded data, A06/C06 |
| [0016](rfcs/RFC-0016-Deterministic-Execution.md) | Execution | Delayed feedback and qualified execution assumptions, A08/B07 |
| [0017](rfcs/RFC-0017-FDIR-and-Fault-Modes.md) | FDIR | Hardware-specific response and evidence, A04/C05 |
| [0018](rfcs/RFC-0018-Python-Worker-SDK-and-Isolation.md) | Python workers | Full-request deadlines, bounded I/O and cleanup, A05 |
| [0019](rfcs/RFC-0019-Deployment-Graph-Validation.md) | Graph validation | Resolved manifest identity and runtime enforcement distinction, A08/B02 |

## Development-only RFCs: review before integration

These identifiers are already used on development `c8aa467`; do not reuse them. They are not files in the main baseline.

| RFC | Development source | Review focus |
|---|---|---|
| 0020 | [Deterministic Vehicle Simulation](https://github.com/KevinBusuttil/neuradix-robotics-platform/blob/c8aa4671bcee8739354beb7880551db7f64314fa/docs/rfcs/RFC-0020-Deterministic-Vehicle-Simulation.md) | Preserve fast fixture, add native backend scope under C01/C02 |
| 0021 | [MCAP Recording Backend](https://github.com/KevinBusuttil/neuradix-robotics-platform/blob/c8aa4671bcee8739354beb7880551db7f64314fa/docs/rfcs/RFC-0021-MCAP-Recording-Backend.md) | Replace private subset/interchange assumptions under A06 |
| 0022 | [Studio Inspection Model](https://github.com/KevinBusuttil/neuradix-robotics-platform/blob/c8aa4671bcee8739354beb7880551db7f64314fa/docs/rfcs/RFC-0022-Studio-Inspection-Model.md) | Reuse headless semantics in graphical shared-service workflow |

## New reserved decision slots

| RFC | Proposed subject | Must decide before |
|---|---|---|
| 0023 | Unified System Model and Project Compiler | B01/B02: model versions, lock identity, target placement and diagnostics |
| 0024 | Target ABI, Wire Layout and Cross-Device Conformance | A02/A03: precision, ordered codecs, compact IDs and migration |
| 0025 | Native Simulation and Studio Service Boundaries | C01/C03: stepping/reset, device binding, import fidelity, shared UI/CLI API |
| 0026 | Durable Runs, Workers and Artifact Identity | D01/D02: leases, retry/cancel, duplicate completion, partial results and retention |
| 0027 | Site Independence and Enterprise Operations | D03/D04/F01: discovery, freshness, access/quotas, backup and HA evidence |

Reserved slots are backlog entries; their RFC files do not yet exist. Each implementation package must submit the needed decision text and alternatives before stabilising its public contract.

## Reserved later-profile identifiers

Preserve 0006 Swarm Membership and Task Allocation; 0007 Federated Shared World Model; 0008 Studio XR Operator Intent and Authority; 0009 Aero Airspace and Collision Safety; 0010 Multi-Vehicle Simulation and Replay; 0011 Flight Profile and Restricted Rust; 0012 Ground/Fleet Identity and Deployment Manifest. These map to F02/F04/F06/F07/F08. The old Studio v0.1 suggestion to use RFC-0006 for Studio conflicts with this reservation and is superseded; Studio service decisions use 0025.

## Decision completion

Record status, owner/reviewer, alternatives, compatibility/migration, failure/timing/security assumptions and acceptance fixtures. Update the specification or plan when a decision changes it. Architecture acceptance and implementation maturity remain separate; link actual implementation/test evidence in [Neuradix_Capability_Status.md](Neuradix_Capability_Status.md).
