# Neuradix capability and evidence status

**Updated for the WP-A04.2 implementation branch.** This register is the current
implementation record. Requirements and future work live in the
[Specification v0.6](Neuradix_Robotics_Platform_Functional_Specification_v0.6.md)
and [Implementation Plan v0.4](Neuradix_Implementation_Plan_v0.4.md).

## Integrated revisions and evidence

| Revision | Observed state |
|---|---|
| [Code baseline on main `d5fbd69`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/d5fbd69b7b901ad8899b9c911ad09e265ff76636) | PR #7 integrates all six development increments and the merged PR #6 fixes; 15 library/tool crates and four executable examples, workspace 0.0.1. |
| [Codec implementation `b05baed`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/b05baed366a91098b0bc0f0401c754d6974b6f46) | Canonical, versioned scalar wire identity and Uno ABI checks; later `7fc5225` only records evidence. |
| [Codec CI](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34248323298) | Host formatting, Clippy, 191 workspace tests/doctests and docs passed; the two AVR tests passed in their separate job. |
| [Integration CI](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34250744255) | Host and AVR jobs passed on PR #7's combined code before integration into main. |
| Historical review | Main `e39da5e` and development `c8aa467` remain the pinned pre-fix assessment in [Review and Strategy](Neuradix_Robotics_Platform_Review_and_Strategy_v1.0.md); they are not the current main/development split. |

## WP-A04 integration and branch increment

[A04.1](implementation/WP-A04.1-Trusted-Evaluation.md) is integrated through
[PR #8](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/8), main
[`b4aae739`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/b4aae73979c620e3b7c674af1623a2b014eae612).
Its merge tree matches the reviewed PR head; [merge CI](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34283480925)
passed. A04.1 records 64 focused tests (a workspace subset), 211 workspace
tests/doctests and both AVR checks, formatting, Clippy, docs and independent
no_std checks.

[A04.2 branch evidence and API migration](implementation/WP-A04.2-Command-Freshness.md)
record bounded shared host/embedded freshness, deadlines, sequence/generation
checks, accepted-command watchdogs and periodic idle expiry. This branch adds
`neuradix-command-core` (16 library/tool crates total), a full metadata binding to
existing serial frames, and round-trippable non-finite rejection lineage. Trusted
durable startup generation allocation and a shared reference clock are explicit
integration requirements. A04.2 is not yet merged. Implementation `41ad541b` passed
[PR CI](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34287037280):
69 focused tests, 216 workspace tests/doctests, both AVR checks, all three migrated
examples, formatting, Clippy, docs and four independent no_std checks. The two
workspace-ignored AVR tests execute separately. Changed-document links pass;
32 pre-existing broken archived links are explicitly recorded in the evidence.
No physical hardware or durable-storage integration was tested. WP-A04 remains
partial; Gate A remains open.

## Capability inventory

| Capability | Integrated scope on main | Remaining work |
|---|---|---|
| Contracts/time/local queues | Scalar schemas, semantic hash, Rust generation, tagged clocks, no_std time and bounded local transport | A02/B01/B03 |
| Embedded codecs | Canonical name-sorted `neuradix.scalar-le.v2`, full wire identity, Rust/C++ generation, required decoder identity, wire manifests and explicit AVR ABI checks | Complete transport binding, compact-ID collisions, recording migration and physical vectors: A02/A03/B05/B06 |
| Execution/control | Lifecycle and input-driven lockstep processor; offline graph validation | Resolved identities, delayed feedback and deployed supervisor: A08/B02/B07 |
| Authority/health | Host scalar gate, lineage and FDIR; embedded gate/watchdog primitives; A04.1 trusted time and numeric invariants | A04.2 branch adds command validity; A04.3 slew alignment and rig evidence remain: A04/B03/C05 |
| Python | JSON-line supervised worker process and failure tests | Bound all I/O, cleanup and resources: A05/B07 |
| Recording/replay | Native recording and digest, handwritten MCAP subset; processor re-execution exists in a runtime test | Independent MCAP interchange and arbitrary program replay: A06/A07/C06 |
| Simulation | Fixed-step one-dimensional closed-loop AUV depth model and example | Native backend API and general simulator integration: C01/C02/C05 |
| Studio | Headless timeline/scalar inspection library and CLI | Graphical authoring, diagnosis and shared services: C03/C04 |
| Embedded runtime/link | Host-tested no_std core and serial CRC/sequence framing; actual AVR compiler checks for generated scalars | Complete firmware, flash/monitor, physical board execution, timing and stack: B04/B05/B06 |
| Networking/interop | Interfaces and plans | Network/shared-memory and ROS/MAVLink adapters: D03/E02 |
| Enterprise/fleet/AI/XR | Specifications and plans | Workers, fleet, model integration and XR: D01–D04/F01–F08 |

## Evidence vocabulary and limits

**Planned** means described only. **Implemented** means executable source with a
stated scope. **Independently interoperable** requires external fixtures/tools
for the claimed exchange. **Hardware-demonstrated** requires actual named
devices. **Supported** requires a maintained matrix, release gates, owner and
compatibility policy. These are separate evidence dimensions.

The independent C++ producer/reordered Rust consumer test validates the generated
scalar codec. It does not establish ROS or general MCAP interoperability.
AVR GCC 7.3.0 compiled/linked the supported ATmega328P harness and rejected
binary64 with the intended diagnostic. The harness uses 3,100 bytes of flash and
328 bytes of static SRAM; stack and hardware timing remain unmeasured.
See [Gate A implementation evidence](implementation/Gate-A-Embedded-Wire-and-ABI.md).

The workspace's two AVR tests are explicitly ignored by the ordinary test command
and executed in the separate AVR CI job. Host C++ and Python are CI prerequisites;
local Python tests still allow skips. No physical board, complete simulator,
hard-real-time profile, fleet scale or enterprise HA claim follows from these tests.

## Findings and closure status

| Finding | Current result | Work package | State |
|---|---|---|---|
| Semantic hash / authored wire order | Canonical v2 layout and separate wire identity implemented; independent reordered-endpoint regression passes | A02 | Generator defect fixed; transport binding, collisions and recording migration remain open |
| AVR binary64 projection | Explicit Uno generation rejects binary64; portable header fails the real AVR compiler ABI guard | A03 | Unsafe projection fixed; physical board vectors, stack and timing remain open |
| Host authority/finite values | A04.1 integrated trusted evaluation time, private validated configuration, safe/final outputs; A04.2 branch adds bounded freshness/deadline/sequence/generation checks | A04 | Partial: trusted startup/board integration, A04.3 slew alignment and rig evidence remain open |
| Worker bounds | Blocking write/wait and unbounded line/channel can escape the deadline | A05 | Open |
| MCAP subset | Private encodings/metadata and unsupported chunks can fail or omit external data | A06 | Open |
| Replay CLI scope | `replay run` verifies the record digest; it does not execute a changed graph/controller | A07 | Existing behavior; extend through an explicit migration |
| Deployment resolution/feedback | Identity excludes resolved behavior; cycle/role validation is not runtime enforcement | A08 | Open |

WP-A01 has an integrated source baseline and compiler evidence. The broader evidence
inventory and optional-tool audit remain. WP-A02 and WP-A03 are partial; their
full acceptance criteria are unchanged. **Gate A remains open.**

## How to update this register

Record the integrated commit, exact scope, toolchain/target, executed tests and
retained evidence. Preserve historical findings with their original revisions,
then record fixes separately. Publish workload limits only after representative
performance and failure experiments; source presence or a passing unit suite
does not establish deployment support.
