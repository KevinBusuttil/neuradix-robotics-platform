# Neuradix capability and evidence status

**Updated for the WP-A08 resolved deployment identity branch.** This register is the current
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

## WP-A04 integration

[A04.1](implementation/WP-A04.1-Trusted-Evaluation.md) is integrated through
[PR #8](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/8), main
[`b4aae739`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/b4aae73979c620e3b7c674af1623a2b014eae612).
Its merge tree matches the reviewed PR head; [merge CI](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34283480925)
passed. A04.1 records 64 focused tests (a workspace subset), 211 workspace
tests/doctests and both AVR checks, formatting, Clippy, docs and independent
no_std checks.

[A04.2](implementation/WP-A04.2-Command-Freshness.md) is integrated through
[PR #9](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/9), main
[`c127c7d`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/c127c7d433b8e5a07f7a7df4b5a96cf6e6ed7a03).
Review corrected stale renewal time reviving expiry and non-finite lineage values
being lost in CLI explanations/series. Corrected head `665fd5d` passed
[CI 34288937294](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34288937294):
71 focused tests, 219 workspace tests/doctests, two AVR checks, three examples,
formatting, Clippy, docs and four independent no_std checks. The shared
`command-core` brings the workspace to 16 library/tool crates. Trusted durable
startup, clock relationship and periodic scheduling remain deployment obligations.

[A04.3 evidence and API migration](implementation/WP-A04.3-Slew-Alignment.md)
record shared host/MCU elapsed-time slew, explicit rate units, inward output
rounding and changing-period conformance. Implementation `56e8688` passed
[CI 34290519286](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34290519286):
88 focused and 236 workspace tests/doctests, two separate AVR checks, three
examples, formatting, Clippy, docs and four independent no_std checks.
[PR #10](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/10) merged
as `9dafdb7` after review of unchanged head `a81fab7` and passing
[current-revision CI](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34290766316).
The evidence document
distinguishes corrected CI failures, unavailable hardware and archived link debt.
No physical hardware, durable-storage or board timing/resource validation is
claimed. WP-A04 remains partial, ACC-05 incomplete and Gate A open.

## WP-A05 integrated I/O, heartbeat and per-process resources

[PR #11](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/11) adds
validated protocol/storage limits, one deadline with cleanup reserve, Linux
nonblocking stdio, process-group cleanup and bounded deferred reaping. Failed
launch attempts consume the restart budget. [Implementation evidence](implementation/WP-A05-Bounded-Worker-IO.md)
records API migration, exact checks, OS scope and remaining acceptance. The PR
merged as `e12420a` after fixing small-limit SDK error reporting in `063d0d77`.
[Current PR CI 34318590924](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34318590924)
passed, including five SDK tests. This establishes integration of that bounded
increment, not full containment. The following counts retain their original revision.
Implementation `08d0f987` passed [PR CI 34293819208](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34293819208)
and its push run: 32 Python-crate tests/doctests, four separate SDK tests,
88 command regressions, 264 workspace tests/doctests, two separate AVR checks,
four examples, four independent no_std checks, formatting, Clippy and docs.
Nested subprocess reports are not double-counted; focused tests overlap workspace.
[PR #12](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/12) adds
[heartbeat health and bounded recovery](implementation/WP-A05-Worker-Heartbeat.md)
and merged as `5cda65a` with unchanged reviewed head `0cd31b7` and passing
[current PR CI 34320795164](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34320795164)
and push CI `34320791968`. Private validated monotonic due/expiry windows, idle probes,
matching replies, latched failure reasons and restart accounting are implemented
on main; final verification is tracked in its evidence document.
Implementation `58e2b224` passed [CI 34320430371](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34320430371)
and its push run: 55 Python-crate tests/doctests, six SDK tests, 88 command
regressions, 287 workspace tests/doctests, two separate AVR checks, four examples,
four independent no_std checks, formatting, Clippy and docs. The PR records
current-revision checks after final documentation/shared-observation changes.
[PR #13](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/13) adds
[Linux per-process CPU-time and address-space limits](implementation/WP-A05-Worker-Resource-Limits.md)
and merged as `741b5bc53ddf293c7d809497018dbc36f8a60b78`.
[Post-merge main CI 34327790906](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34327790906)
passed. This verified status correction does not change A05 implementation.
A required trusted native launcher installs and verifies
private validated limits before Python executes; auditable setup and exit results
preserve existing bounded supervision. Actual Linux enforcement and current
verification are recorded in that evidence document. Implementation `4742a1ce`
passed [PR CI 34324405885](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34324405885)
and push CI `34324402352`: 75 Python-crate tests/doctests (16 resource scenarios),
six SDK tests, 88 command regressions, 307 workspace tests/doctests, two separate
AVR checks, four examples, four independent no_std checks, formatting, Clippy
and docs. Counts exclude repeated subprocess reports. Aggregate process-tree
CPU/RSS, GPU limits, escaped-session containment, other OS qualification and
deployment supervision remain open. WP-A05/ACC-09 are partial; Gate A stays open.

## WP-A06 import and uncompressed writer integrated

[PR #14](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/14)
merged as `79286d1e16280fc3ecadb7fa94dc3e8f98af9fa3` with successful
[post-merge CI 34334529747](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34334529747).
Bounded maintained import and independent uncompressed/chunked/LZ4 producer
fixtures are integrated; see [import evidence](implementation/WP-A06-Bounded-MCAP-Import.md).

[PR #15](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/15),
merged as `3e47e7f973445d381473b3d35c1c9ee14452187a` with passing
[main CI 34350022456](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34350022456), adds validated uncompressed streaming, direct borrowed
payload writes, bounded definition/finalization state, CRCs and a correctly grouped
Statistics-only summary. Errors latch and only explicit successful finish permits
publication. CLI output uses a provisional File. Independent export decoding and
measured memory are tracked in [writer evidence](implementation/WP-A06-Bounded-MCAP-Writer.md).
Implementation `8674da3` passed [CI 34337776482](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34337776482):
44 recording and 336 workspace tests/doctests, independent export/CRC agreement,
all preserved conformance suites and writer RSS 3,024/3,128 KiB for 8/128 MiB payloads
against 64 MiB ceilings. Final-revision checks are tracked by the PR.
PR #14 review follow-ups bound channel lookup work and reject incomplete statistics;
the maintained parser's existing duplicate-key rejection has regression coverage.
WP-A06/ACC-08 remain partial pending broader interchange/scale
qualification. A04/A05 and Gate A retain their remaining acceptance criteria.

## WP-A07 single-processor replay integrated

[PR #16](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/16), merged as `88c8eb1015a96d5314b1314b623758c7d43100f1` after current-head review and passing PR CI 34380936422/push CI 34380929961,
adds `neuradix-runtime::replay` with bounded owned cases, immutable configuration/seed,
a separate evaluation schedule and original source timestamps. Each run calls a
selected factory and fresh processor; fixed-size error evidence and capped mismatch
diagnostics report exact per-tick comparisons. No output payload history is retained.
The executable example demonstrates unchanged-program agreement and changed-program
failure. The existing CLI still verifies record integrity. See
[A07 evidence](implementation/WP-A07-Program-Replay-Runner.md).
Pinned implementation CI 34380370682 passed: 346 workspace tests/doctests, 20 runtime tests/doctests, preserved A04/A05/A06 and independent MCAP checks, SDK, no_std and separate AVR conformance. WP-A07/ACC-07 remain partial; arbitrary graphs and closed-loop evidence are deferred.
WP-A01's integrated baseline supplies the prerequisite, while its broader inventory
remains partial. A04/A05/A06 and Gate A retain their open criteria.

## WP-A08 resolved deployment identity branch

The current branch adds versioned declared and resolved identities, bounded canonical
configuration and shared scalar layout computation. All declared ports resolve;
unsupported layouts and invalid graphs cannot produce a resolved identity. Immutable
reports prevent caller edits from forging successful validation. Identity labels do
not attest execution or grant authority. API/CLI pin migration is deliberate; see
[A08 evidence](implementation/WP-A08-Resolved-Deployment-Identity.md).
Verification and PR integration are pending. WP-A08 stays partial for delayed
feedback and runtime authority enforcement. Existing package/acceptance gaps remain.

## Capability inventory

| Capability | Integrated scope on main | Remaining work |
|---|---|---|
| Contracts/time/local queues | Scalar schemas, semantic hash, Rust generation, tagged clocks, no_std time and bounded local transport | A02/B01/B03 |
| Embedded codecs | Canonical name-sorted `neuradix.scalar-le.v2`, full wire identity, Rust/C++ generation, required decoder identity, wire manifests and explicit AVR ABI checks | Complete transport binding, compact-ID collisions, recording migration and physical vectors: A02/A03/B05/B06 |
| Execution/control | Lifecycle and input-driven lockstep processor; offline graph validation | Resolved identities, delayed feedback and deployed supervisor: A08/B02/B07 |
| Authority/health | Host scalar gate, lineage and FDIR; embedded gate/watchdog primitives; A04.1 numeric invariants, A04.2 command validity and A04.3 physical-unit slew | Board integration and rig evidence: A04/B03/C05 |
| Python | Bounded Linux I/O, cleanup, admission/reaper, heartbeat/recovery and per-process CPU/AS through PRs #11–#13 | Aggregate resources, OS qualification and deployment remain: A05/B07 |
| Recording/replay | Native recording/digest and integrated bounded MCAP import; processor re-execution exists in a runtime test | A06 writer integrated; A07 single-processor runner integrated; broader interchange/scale and graph/closed-loop replay remain: A06/A07/C06 |
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
older Python tests still allow local skips; the A05 adversarial/SDK suites require
Python, and CI requires the interpreter before testing. No physical board, complete simulator,
hard-real-time profile, fleet scale or enterprise HA claim follows from these tests.

## Findings and closure status

| Finding | Current result | Work package | State |
|---|---|---|---|
| Semantic hash / authored wire order | Canonical v2 layout and separate wire identity implemented; independent reordered-endpoint regression passes | A02 | Generator defect fixed; transport binding, collisions and recording migration remain open |
| AVR binary64 projection | Explicit Uno generation rejects binary64; portable header fails the real AVR compiler ABI guard | A03 | Unsafe projection fixed; physical board vectors, stack and timing remain open |
| Host authority/finite values | A04.1 integrated trusted evaluation time, private validated configuration, safe/final outputs; A04.2 integrated bounded command validity; A04.3 integrated physical-unit slew | A04 | Partial: trusted startup/board integration, physical safe response and timing/resource evidence remain open |
| Worker bounds | PRs #11/#12 integrate bounded Linux I/O/cleanup and heartbeat/recovery; PR #13 adds per-process CPU/AS limits | A05 | Partial: PR #13 integrated; aggregate resources and additional OS/deployment work remain |
| MCAP subset | PR #14 integrates bounded maintained import; PR #15 integrates bounded uncompressed output and independent export checks | A06 | Partial: bounded import/writer integrated; broader interchange/scale qualification remains |
| Replay CLI scope | `replay run` verifies the record digest; separate A07 runner invokes one selected Processor with exact comparison | A07 | Runner integrated; graph execution and ACC-07 closed-loop evidence remain open |
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
