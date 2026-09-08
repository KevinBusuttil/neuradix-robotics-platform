---
title: "Neuradix Studio — Integrated Engineering Plan"
author: "Busuttil Technologies Limited"
date: "8 September 2026"
version: "0.2 Draft"
status: "Proposed documentation baseline; implementation status is separate"
supersedes: "Neuradix_Studio_Implementation_Plan_v0.1.md"
---

Aligned to [Functional Specification v0.6](Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) and [Detailed Implementation Plan v0.4](Neuradix_Implementation_Plan_v0.4.md). The master plan owns dependencies, estimates and release dates; [Capability Status](Neuradix_Capability_Status.md) owns current evidence.

# 1. Product outcome

Studio is the integrated environment for modelling, programming, simulating, deploying and diagnosing a robotic system. The first integrated alpha includes **inspection and minimal authoring**. It serves embedded/control, robotics, simulation, test and operations engineers; AI assistance is optional.

The initial implementation is a local web UI backed by Rust application/query services shared with the CLI. This supersedes the earlier requirement to prove identical WASM/native rendering, a desktop wrapper and XR before the basic workflow is complete. Those remain possible implementations or extensions, subject to measured need and supported-device evidence.

The current development branch has a headless inspection library/CLI only. No graphical Studio, editor, live graph supervisor or complete simulator integration is established. Reuse reviewed query semantics; do not confuse library presence with a delivered user interface.

# 2. Architecture and boundaries

| Layer | Responsibility |
|---|---|
| Project model/compiler | Own schemas, targets, bindings, validation and resolved identities |
| Shared application services | Execute build, flash, launch, simulation/run operations and queries; return structured results |
| Rust query/data services | Read/index/decimate recordings and live data within explicit memory/query limits |
| Web UI | Project navigation, forms, graph/scene views, signal/timeline panels and task progress |
| Optional shells/renderers | Desktop, native/WASM acceleration and XR after qualified implementation decisions |

The UI must not implement a second contract, authority or clock model. Reuse the core definitions through shared services or generated API types. Place large-data work on appropriate native/worker services initially; browser memory limits and renderer support require explicit measurement. Exact framework/renderer selection is an implementation decision with a bounded spike, not a prerequisite for the platform architecture.

# 3. Integrated alpha experience

1. Open one project and see its robot, components, targets, configuration and asset versions.
2. Edit controller parameters, basic scene properties and virtual/physical device bindings.
3. Validate through the project compiler; explain unit/frame, ABI, memory and placement errors.
4. Run/step/reset simulation and inspect signals, controller state, health and command lineage.
5. Build and flash the selected Uno/MCU through the same services as the CLI.
6. Identify active firmware/configuration; launch and observe the physical system.
7. Capture a failure, link it to source/run identities and rerun an actual program regression.
8. Submit the same scenario manifest to local or server workers when Gate D is available.

Simple forms and a structured project editor are sufficient for the first authoring workflow. Full visual programming, advanced CAD editing and collaborative scene editing are separate increments. Generated outputs must not overwrite handwritten component code.

# 4. Work packages and acceptance

| Package | Minimum deliverable | Evidence |
|---|---|---|
| C03 | Local UI; project/graph, signals, timeline, frames/clocks, health, identity and lineage | Bounded live/recorded inspection; UI loss has no control effect |
| C04 | Parameter/scene/binding authoring and compiler/build/flash/launch integration | One project completes the same workflow through CLI and Studio |
| C06 | Captured incident and program-replay comparison views | Changed controller actually executes; replay/outcome claims are distinct |
| D01/D04 | Worker submission, run status/artifacts and scoped project/site access | Failed/retried/cancelled jobs and access boundaries are visible |
| E03/E05 | Installable documented workflow and independent engineer trials | Clean-machine reproduction and observed task completion |
| F07 | Qualified XR pack | Shared state/authority, headset loss and measured comfort criteria |

Use the master plan dependencies/effort ranges and ACC-01/07/10/11/12/13/16. XR is not a Gate C or core 1.0 acceptance condition.

# 5. State, timing and authority presentation

Always label live, simulated, replayed, estimated, predicted and stale data where present. Show source clock/domain, age and uncertainty rather than presenting all timestamps as interchangeable. Display units/frames from validated contracts and explicit conversions. A command view links proposal, authority decision, constraints and applied output.

Firmware identity, reset/watchdog state, resource budgets and communication faults belong in normal device inspection. Separate configured budgets from observed measurements. Show deployment preview and active versus proposed artifact/configuration before operational changes.

Studio uses the declared authenticated/audited operation path and cannot bypass local authority. Disconnecting or overloading the UI must not stop control or suspend a watchdog. Programmatic CLI/UI operations use the same permission and validation model.

# 6. Performance and data access

Benchmark first-useful-view time, timeline scrub/query latency, working-set memory, dropped/display-decimated samples and scene frame stability on named workloads and hardware. Distinguish visualization decimation from lost recorded data. Use bounded subscription queues, query cancellation, indexed/chunked reads and backpressure before expanding dataset size.

The earlier broad performance and competitor-superiority assertions are design aspirations requiring evidence. External viewers can provide useful integrations; Studio's differentiation is the coherent project/simulation/deployment/diagnostic workflow. No external viewer is a mandatory robot-control dependency.

# 7. Validation and extension decisions

Before Gate C, an independent engineer should edit a conventional controller, simulate it, flash a real board, inspect a failure and rerun the case through documented steps. Record friction and fix repeated problems before adding panels.

Select native/WASM/desktop rendering when a measured workload justifies it; preserve semantics without promising identical runtime capabilities. XR begins with one actual device/runtime and explicit comfort/fallback criteria. AI assistance submits ordinary validated edits and evidence-linked explanations; it must not become a requirement for ordinary operation.
