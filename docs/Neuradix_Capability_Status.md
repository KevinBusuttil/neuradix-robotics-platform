# Neuradix capability and evidence status

**Verified source baseline: 8 September 2026.** This register describes source/tests and historical CI, not fresh execution. Update it with each integrated implementation PR and evidence artifact. Planned behaviour belongs in [Neuradix_Robotics_Platform_Functional_Specification_v0.6.md](Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) and [Neuradix_Implementation_Plan_v0.4.md](Neuradix_Implementation_Plan_v0.4.md).

| Revision | Observed state |
|---|---|
| [main `e39da5e`](https://github.com/KevinBusuttil/neuradix-robotics-platform/tree/e39da5e31709259b3fd876a9ed2fd350263c2bb3) | 10 library/tool crates, two executable examples; workspace 0.0.1 |
| [development `c8aa467`](https://github.com/KevinBusuttil/neuradix-robotics-platform/tree/c8aa4671bcee8739354beb7880551db7f64314fa) | `claude/gifted-albattani-i7t2wc`, six commits ahead/zero behind main; five additional crates and two examples |
| CI | Historical [main workflow](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/29661906798) and [development workflow](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/29723577698) succeeded; tool-dependent checks may skip |
| This change | Documentation and historical documentation-generator maintenance only; no runtime or firmware fixes |

| Capability | main | Additional development evidence | Next work |
|---|---|---|---|
| Contracts/time/local queues | Scalar schemas, semantic hash, Rust codegen, clock tags, bounded in-process transport | no_std time and fixed scalar Rust/C++ projections | A02/A03/B01/B03 |
| Execution/control | Lifecycle and input-driven lockstep processor; offline graph validation | No deployed graph supervisor | A08/B02/B07 |
| Authority/health | Scalar gate, lineage and FDIR | Separate embedded gate/watchdog primitives | A04/B03/C05 |
| Python | JSON-line supervised process and failure tests | No stronger resource/IO bounds established | A05/B07 |
| Recording/replay | Native recording and digest; program re-execution exists in a runtime test | Handwritten MCAP subset | A06/A07/C06 |
| Simulation | Canned depth stream/controller example | Fixed-step one-dimensional closed-loop AUV depth model | C01/C02/C05 |
| Studio | CLI inspection/explain | Headless timeline/scalar inspection library and CLI; no graphical UI | C03/C04 |
| Embedded | Design documents | Host-tested no_std core, serial framing, generated projections | B04/B05/B06; no actual board build/flash evidence |
| Networking/interop | Interfaces and plans | No network/shared-memory or ROS/MAVLink implementation established | D03/E02 |
| Enterprise/fleet/AI/XR | Specifications and plans | No worker cluster, fleet, model integration or XR implementation established | D01–D04/F01–F08 |

## Evidence vocabulary

**Planned**: described only. **Implemented**: executable source exists with a stated scope. **Independently interoperable**: external fixtures/tools verify exchange. **Hardware-demonstrated**: actual named device tests exist. **Supported**: maintained version matrix, required release tests, owner and compatibility policy exist. These are evidence dimensions; a distribution's Experimental/Preview/Supported/Qualified-by-project label does not replace the evidence.

No board, broad simulator, ROS bridge, hard-real-time profile, large-fleet limit or enterprise HA claim is established by this review. Native no_std compilation and host C++ tests are not actual board tests.

## Open static findings

| Finding | Trigger and consequence | Work package | State |
|---|---|---|---|
| Semantic hash / wire order | Field-order-independent identity with declaration-order offsets can exchange equal-width values silently | A02 | Open; no compiled reproduction in this review |
| AVR binary64 projection | Generated eight-byte memcpy uses double; standard Uno double is four bytes | A03 | Open; source/ABI finding, no AVR execution |
| Host authority/finite values | Sender evaluation timestamp and insufficient numeric/configuration validation | A04 | Open; static finding |
| Worker bounds | Blocking write/wait and unbounded line/channel escape the request deadline | A05 | Open; static finding |
| MCAP subset | Private encodings/metadata and unsupported chunks can fail or omit external data | A06 | Open; static finding |
| Replay CLI scope | `replay run` checks record digest; does not execute changed graph/controller | A07 | Existing behaviour; document and extend deliberately |
| Deployment resolution/feedback | Identity excludes resolved behaviour; blanket cycle/role rules do not implement runtime enforcement | A08 | Open; static finding |

The [Review and Strategy](Neuradix_Robotics_Platform_Review_and_Strategy_v1.0.md) links each finding to pinned source. Fixing documentation does not close these items. Required closure evidence is in the work package and ACC catalogue.

## How to update this register

Record the integrated commit, exact capability scope, toolchain/target, tests actually run and retained evidence. Keep development-only work explicitly separate until integrated. Publish measurable workload limits only after representative performance/failure experiments; do not derive support from file counts, no_std use or generic CI success.
