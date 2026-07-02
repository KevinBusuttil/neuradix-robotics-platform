---
title: "Neuradix Studio — Implementation Plan"
subtitle: "A contract-native, high-performance robotics observability product"
author: "Engineering"
date: "23 June 2026"
version: "0.1 Draft"
status: "For review"
---

# 0. Purpose and decisions taken

This document plans **Neuradix Studio** as a *first-class, differentiating product*
within the platform — an equivalent to, and improvement on, Foxglove — rather than a
deferred afterthought. It refines spec §32 and slots into the
[Implementation Plan](./Neuradix_Implementation_Plan_v0.1.md).

Decisions ratified for this plan:

| Decision | Choice | Rationale |
|---|---|---|
| **What makes it "better"** | **(a) Neuradix-native semantics** and **(b) performance & scale** | Foxglove sees generic messages; Studio understands contracts, units, frames, lineage, authority and health. Plus it must stay smooth on multi-GB logs and high-rate streams. |
| **Sequencing** | **Inspection-first, grow alongside core** | Read-only inspection views first (graph, signals, lineage, replay); authoring views later. Foxglove remains a stopgap until Studio reaches parity. Matches spec §42 risk guidance. |
| **Platform** | **Web local-first + optional desktop wrapper + XR headsets** | Browser app that runs fully offline (spec §32.1); Tauri desktop wrapper for native performance and filesystem access; **immersive 3D on XR headsets** via WebXR (browser) and OpenXR (native) from the same engine. |

The defining architectural consequence of "web local-first **and** high performance"
is a **shared Rust engine compiled to both WASM and native**, rendering via **wgpu**.
The browser and the desktop wrapper run the *same* engine code; only the shell differs.

---

# 1. Positioning: why this can beat Foxglove

Foxglove is excellent but **schema-generic** — it visualizes messages without
understanding their meaning. Neuradix Studio links directly against the platform's
Rust crates, so it can do things a generic tool structurally cannot:

| Capability | Foxglove (generic) | Neuradix Studio (native) |
|---|---|---|
| Units on plot axes | Manual / guessed | **Read from the contract** (§10, §15) — axes auto-labelled, unit-mismatch flagged |
| Coordinate frames | Manual TF config | **Transform service-aware** (§15.3): time-indexed lookup, uncertainty, duplicate-authority warnings |
| Data staleness | Not modelled | **`maximumAge` from contract** (§10.3) — stale samples visibly flagged |
| Clock domains | Single timeline assumed | **Multi-domain timeline** (§14.1): monotonic vs UTC vs sim vs replay shown distinctly |
| Why did the robot do X? | Not possible | **Causal command lineage / `explain`** (§25.3–§25.4): click an actuator command → see the sensor→estimator→planner→authority→safety chain |
| Safety decisions | Invisible | **Authority leases + constraint modifications** (§16) rendered inline on the command timeline |
| Determinism | Not a concept | **Lockstep / branch / counterfactual replay** (§24.4) driven from the replay clock |
| Health | Generic topic | **Structured health model** (§25.5): healthy/degraded/unhealthy/unavailable/unknown with reasons + recommended action |
| Provenance | None | **Schema hashes, component versions, manifest hash, SBOM** surfaced from the recording (§24.2) |

These all come "for free" from reusing the core crates — they are the moat, and they
are exactly what an external tool can never replicate without reimplementing Neuradix.

**Engineering note:** This is also the reason Studio must depend on the *same*
`contracts`, `record`, `time`, and `data-plane` crates — not a parallel reimplementation.
Any drift between Studio's understanding of a contract and the runtime's is a bug
factory. Studio is, architecturally, "the runtime's data model rendered."

---

# 2. Product principles

Inherited from spec §32.1 and §32.3, made concrete:

1. **Local-first and fully offline.** No internet, no account, no cloud dependency to
   open a recording or inspect a live robot (§32.1). Sharing/collaboration is additive,
   never required.
2. **Read-only by default; commanding is privileged.** Studio is an engineering tool
   and MUST NOT silently acquire operational authority (§7.7, §32.3). Live actuator
   commands are disabled by default and, when enabled, go through Ground authority
   services with authentication, confirmation, and audit.
3. **Studio disconnection never affects the robot** (§32.3). It is an observer.
4. **The same engine everywhere.** Browser (WASM) and desktop (native) run identical
   Rust engine code; no feature forks between them.
5. **Performance is a feature, budgeted and benchmarked**, not hoped for (see §5).
6. **Inspection before authoring.** Ship the read-only views that make the platform
   debuggable first; deployment/mission/scenario *editing* comes after (§42).

---

# 3. Architecture

```text
┌──────────────────────────────────────────────────────────────────┐
│  Shell                                                             │
│   • Browser (PWA, offline cache)        • Desktop wrapper (Tauri)  │
│     served locally / from registry        native FS, menus, GPU    │
│   • XR: WebXR (browser headsets)        • XR: OpenXR (native)      │
└───────────────┬───────────────────────────────────┬──────────────┘
                │                                     │
        ┌───────▼─────────────────────────────────────▼─────────┐
        │  Web UI layer  (TypeScript + React)                    │
        │   • Dockable panel/layout system                       │
        │   • Panel chrome, controls, forms, command palette     │
        │   • Talks to engine via a typed wasm-bindgen interface │
        └───────────────────────┬────────────────────────────────┘
                                 │  (zero-copy where possible)
        ┌────────────────────────▼───────────────────────────────┐
        │  studio-engine   (Rust → WASM + native)                 │
        │   • MCAP read/index/decimate (reuse `record`)           │
        │   • Contract & schema awareness (reuse `contracts`)     │
        │   • Time/clock-domain handling (reuse `time`)           │
        │   • Frame/transform resolution (reuse `frames`)         │
        │   • Lineage / explain query                             │
        │   • Decode pipelines (image/pointcloud/tensor/sonar)    │
        │   • wgpu renderer (3D scene, GPU plots)                 │
        │   • Live client (data-plane subscriber)                 │
        └───────────────┬───────────────────────────┬────────────┘
                        │ files (MCAP)               │ live stream
        ┌───────────────▼──────┐        ┌────────────▼─────────────┐
        │  Recordings (MCAP +  │        │  studio-backend (opt.)   │
        │  manifest, schemas,  │        │  • Local bridge to a live │
        │  hashes) §24.2       │        │    Neuradix graph         │
        └──────────────────────┘        │  • WebSocket: Neuradix-   │
                                        │    native + Foxglove-compat│
                                        │  • Reuses data-plane/record│
                                        └───────────────────────────┘
```

## 3.1 Why a shared Rust engine (the key decision)

- **One source of truth for semantics.** The engine links the real `contracts`,
  `time`, `frames`, and `record` crates, so Studio's understanding of a stream is, by
  construction, identical to the runtime's.
- **Performance.** MCAP indexing, decimation, decode, and frame math run in Rust, not
  JavaScript. In the browser this is WASM; in the desktop wrapper it is native (faster
  still, with threads and full GPU).
- **wgpu renders everywhere.** `wgpu` targets Vulkan/Metal/DX12 natively and
  WebGPU (with WebGL2 fallback) in the browser, from one Rust codebase. 3D scenes and
  high-rate plots are GPU-rendered rather than DOM/SVG-bound.
- **No feature fork.** Browser and desktop differ only in the shell (file access,
  windowing), never in capability.

## 3.2 Technology choices (build-vs-buy *within* Studio)

| Concern | Use | Don't build |
|---|---|---|
| GPU rendering (3D + plots) | **wgpu** (Rust, native + WebGPU) | a renderer per platform |
| Recording I/O | reuse Neuradix **`record`** (MCAP) | a second MCAP reader |
| Semantics | reuse **`contracts`/`time`/`frames`** | a parallel schema model |
| Web↔WASM glue | **wasm-bindgen + wasm-pack** | hand-rolled FFI |
| Desktop shell | **Tauri** (Rust, small footprint) | Electron (heavy) unless needed |
| UI framework | **React + TypeScript** (mature panel/layout ecosystem) | a bespoke UI toolkit |
| Dockable layout | a maintained docking lib (e.g. dockview-class) | a layout engine from scratch |
| Live transport to browser | **WebSocket**; implement **Foxglove WS protocol** for interop **and** a richer Neuradix-native channel | a new wire protocol first |
| Point-cloud/mesh decode | Rust crates in-engine | JS decoders |

**Interop hook:** implementing the Foxglove WebSocket protocol in `studio-backend`
means existing Foxglove instances can connect to a live Neuradix graph *and* Studio can
read anything Foxglove can — easing migration in both directions and de-risking the
"inspection-first, Foxglove as stopgap" sequencing.

---

# 4. Differentiator 1 — Neuradix-native semantics (the moat)

Concrete features, each enabled by reusing a core crate:

1. **Contract-aware panels.** Selecting a stream shows its contract: type, schema
   version + content hash, unit, frame, clock domain, expected rate, `maximumAge`,
   reliability/queue policy (§10.1). Plot axes are auto-labelled with units; a
   unit/frame mismatch between a plotted pair is flagged, not silently rendered.
2. **Staleness + validity overlay.** Samples older than the contract's `maximumAge`
   (§10.3) are visibly marked; the timeline shows validity intervals.
3. **Multi-clock-domain timeline.** Monotonic, UTC, sensor-hw, sim, and replay times
   are distinguishable (§14.1); the scrubber can pivot on the authoritative timestamp
   declared by the contract (§14.2).
4. **Coordinate-frame viewer with the real transform service** (§15.3): time-indexed
   lookup, interpolation policy, transform uncertainty, graph validation, and
   duplicate-authority detection rendered as warnings.
5. **Causal lineage / `explain` view (flagship).** Click any actuator command on the
   timeline and Studio renders the chain from §25.3: originating sensor samples →
   estimator outputs → planner/controller decision → **authority decision** → **safety
   constraint result** → final actuator command. This is the visual form of
   `neuradix explain command ... --at <t>` (§25.4) and is the single most compelling
   demo of the platform's thesis.
6. **Safety/authority overlay.** Authority leases (holder, scope, expiry — §16.3) and
   constraint modifications/rejections (§16.4, with the responsible rule — §7.5
   NRX-SAF-003) shown inline on the command timeline.
7. **Structured health panel** (§25.5): healthy/degraded/unhealthy/unavailable/unknown
   with reasons, evidence, timestamp, recommended action — not a raw topic dump.
8. **Provenance panel.** Schema hashes, component versions, manifest hash, hardware
   inventory, and SBOM read straight from the recording (§24.2) — turns any recording
   into an auditable artifact.
9. **Uncertainty rendering.** Covariance/uncertainty from the contract (§10.1, §15.2)
   drawn as ellipses/error bars rather than ignored.

---

# 5. Differentiator 2 — Performance & scale

Targets (to be validated by Studio's own benchmark suite, mirroring spec §35):

| Scenario | Target |
|---|---|
| Open a multi-GB MCAP | First frame visible in seconds via **lazy chunk indexing**, not full load |
| Timeline scrub | 60 fps with bounded memory regardless of file size |
| High-rate plots | Smooth on MHz-class signals via **GPU plotting + LOD/decimation** |
| Point clouds / sonar / meshes | Millions of points at interactive frame rates (GPU buffers, frustum culling) |
| Live streaming | Low-latency subscribe with **backpressure**; Studio never stalls the robot (§32.3) |
| Memory | Bounded working set; out-of-core access to large recordings |

Techniques:
- **Lazy, indexed MCAP access** (seek to chunks; never load whole files).
- **Multi-resolution decimation / LOD** for plots and clouds; render summaries when
  zoomed out, raw when zoomed in.
- **GPU-resident buffers** via wgpu for point clouds, images, and plot vertices.
- **Web Workers + threaded native** for decode/index off the UI thread.
- **Virtualized panels and lists** so off-screen work is not done.
- **Zero-copy WASM↔JS** for large buffers where the boundary allows.

**Engineering note:** Performance is mostly *won or lost in the engine and data
access layer*, not the React layer. Keep heavy data out of JavaScript entirely; the UI
layer should only ever hold view state and small derived summaries.

---

# 6. Immersive 3D and XR headsets

Studio targets **XR headsets** (VR/AR/MR — e.g. Meta Quest, Apple Vision Pro, and
other OpenXR devices) as a first-class immersive view, not a bolt-on. The same
`studio-engine` + wgpu architecture makes this natural: stereo rendering and head/hand
tracking are an *additional output + input path* on the existing renderer, not a
separate application.

## 6.1 Why XR fits the existing architecture

- **One engine, one renderer.** wgpu already produces the GPU scene for the desktop
  and browser 3D viewers. XR adds stereo cameras, per-eye projection, and a head pose
  — it reuses the same scene graph, decode pipelines, and GPU buffers (§3.1, §5).
- **Two standards, same code path:**
  - **WebXR** in the browser shell — works on headset browsers (Quest Browser,
    visionOS Safari) and keeps the local-first, install-free promise (§32.1).
  - **OpenXR** in the native/Tauri shell — for maximum performance, lower latency,
    and devices/features not yet exposed to WebXR.
- **Local-first holds.** An engineer can put on a headset and inspect a recording or a
  live robot with no cloud dependency.

## 6.2 What XR is actually *good for* (not novelty)

XR earns its place only where immersion gives real engineering value:

- **Spatial debugging at true scale.** Stand inside the robot's point cloud / sonar
  volume / occupancy map (§11.2, §32.2); judge obstacle geometry, sensor coverage, and
  frame alignment (§15.3) in a way flat screens cannot convey.
- **Coordinate-frame and transform intuition.** See frames, uncertainty ellipsoids,
  and transform trees in 3D space around you (§15.2–§15.3).
- **Replay "walk-through".** Scrub a recorded or lockstep/branch replay (§24.4) while
  physically moving the viewpoint through the scene; watch the causal/safety lineage
  (§4.5) play out spatially.
- **Telepresence inspection.** A remote engineer co-located (in MR) with a live robot's
  reconstructed environment, strictly read-only (§32.3) — Studio disconnection never
  affects the robot, and commanding stays Ground-authenticated and disabled by default.
- **Digital-twin / sim review** of marine and space scenarios (§23, §21.13) at scale.

## 6.3 Constraints and rules

- **XR is an alternate view of the same data, never a separate data model.** It renders
  the engine's existing scene; no XR-only ingestion path.
- **Read-only and observer-safe by default** (§32.3). Any in-XR command capability is
  the same privileged, Ground-authenticated, audited path as elsewhere — and harder
  still to trigger accidentally (deliberate confirmation gestures).
- **Graceful degradation.** No headset → the normal flat 3D viewer; WebGL2 fallback
  where WebGPU/WebXR is unavailable (§9 risk table).
- **Performance budget is stricter.** XR demands sustained 72–90+ fps stereo; the
  LOD/decimation, GPU-resident buffers, and out-of-core access from §5 are mandatory,
  not optional, in this mode. Foveation/fixed-foveated rendering used where available.
- **Ergonomics/comfort** (frame-rate stability, motion comfort) are explicit
  acceptance items, not afterthoughts.

# 7. View / panel catalogue (from spec §32.2), sequenced

Read-only **inspection** views first (the platform-debugging core), then authoring.

**Wave A — Inspection MVP (with core Phases 1–2)**
- Live component graph (nodes, ports, health) — §32.2
- Stream/state browser + raw message inspector
- Time-series plots (GPU)
- Image/video viewer
- Logs, metrics, traces (OpenTelemetry-aligned — §25.1)
- Recording open + timeline scrub + basic replay controls

**Wave B — The differentiators (with core Phase 2–3)**
- Causal command lineage / `explain` viewer ⭐ (§25.3–§25.4)
- Coordinate-frame viewer (transform service) (§15.3)
- Point-cloud and map viewer (§32.2)
- Latency/bandwidth heat map (§32.2)
- Safety & permission view (leases, constraints, decisions) (§16, §32.2)
- Health dashboard (§25.5)
- Replay: lockstep / step / branch / counterfactual (§24.4)
- Fault-injection panel (§34.3)
- **Immersive XR scene view** (point cloud / map / frames in headset) — see §6

**Wave C — Authoring & ops (with/after core Phase 3)**
- Deployment editor (§32.2) — gated by the graph compiler (§28.2)
- Mission/scenario editor (§23.3)
- Package & driver manager (§27)
- Update & deployment status (§26.5)
- Optional, privileged live command (Ground-authenticated) (§32.3)

---

# 8. Phased build plan (mirrors core phases)

| Studio phase | Lands with core | Deliverable | Exit criteria |
|---|---|---|---|
| **S0 — Spikes** | core Phase 0 | wgpu plot + 3D point cloud rendering in **both** browser (WASM) and Tauri from one engine; lazy MCAP open of a large file | one engine renders the same scene natively and in-browser at target fps; multi-GB file opens lazily |
| **S1 — Inspection MVP** | core Phase 1 | Wave A panels; open MCAP + live via Foxglove-compat WS; contract-aware axes/units | a Neuradix recording is fully inspectable in Studio without Foxglove |
| **S2 — Native semantics** | core Phase 2 | `explain`/lineage viewer, frames, safety overlay, health, provenance | the flagship: click a command → see its full causal+safety chain |
| **S3 — Replay & parity** | core Phase 3 | lockstep/branch/counterfactual replay, fault-injection, heat maps; **Foxglove parity for marine demo** | the §38.3 AUV mission is fully debuggable + replayable in Studio; Foxglove no longer required |
| **S4 — Authoring & ops** | post-1.0 | deployment/mission editors, package manager, privileged command | graph-validated editing; audited Ground-authenticated commanding |

**Foxglove exit:** Foxglove remains the stopgap through S1; from **S3** (parity for the
marine reference) Studio becomes the default and Foxglove is optional/interop-only.

---

# 9. Repository structure (extends spec §36 `studio/`)

```text
studio/
  engine/        # Rust: studio-engine (→ WASM + native); deps on core crates
                 #   incl. wgpu renderer + XR session (WebXR + OpenXR) module
  frontend/      # TypeScript + React; wasm-bindgen interface to engine
  backend/       # Rust: live bridge; Foxglove-compat + Neuradix-native WS
  desktop/       # Tauri shell (also hosts native OpenXR sessions)
  benchmarks/    # Studio perf suite (load, scrub, plot, cloud, live, XR stereo)
```

`engine` depends on `crates/contracts`, `crates/time`, `crates/frames`,
`crates/record`, `crates/data-plane` — **never** a fork of them.

---

# 10. Risks (Studio-specific)

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Building a flagship Studio pulls senior effort off the core runtime | **High** | High | Inspection-first; reuse core crates (Studio is thin over them); strict S-phase gating to core phases |
| WebGPU maturity/availability in target browsers | Med | Med | wgpu **WebGL2 fallback**; desktop (Tauri/native) path always available |
| WASM performance/memory ceiling for huge logs | Med | Med | Out-of-core lazy access; push heavy work native in desktop; bounded working set |
| Reimplementing semantics instead of reusing crates (drift) | Med | High | Hard rule: `engine` links core crates; CI check that Studio shares the contract model |
| Scope sprawl into authoring before inspection is solid | Med | Med | Wave A/B before Wave C; authoring gated by the graph compiler being ready |
| Maintaining Foxglove-compat protocol as it evolves | Low | Low | Treat as interop convenience, not a dependency; Neuradix-native channel is primary |
| XR scope/cost (devices, comfort, sustained 90 fps) pulls effort early | Med | Med | XR is an *added output path* on the existing renderer; ship flat 3D first, XR in S3+; reuse §5 perf work |
| WebXR maturity varies across headsets (Quest vs visionOS) | Med | Med | Native **OpenXR** path via Tauri as the high-fidelity fallback; feature-detect and degrade to flat 3D |
| XR motion comfort / fatigue for long debugging sessions | Med | Low | Comfort budget (stable fps, foveation, seated/standing modes) as acceptance items; XR is complementary to flat views, not mandatory |

---

# 11. Acceptance criteria

Neuradix Studio reaches "Foxglove-equivalent-and-better" when:

1. It opens and smoothly inspects a multi-GB MCAP recording fully offline.
2. It connects to a live Neuradix graph (native and Foxglove-compat) without affecting
   robot operation.
3. Plot axes, frames, clock domains, and staleness are driven by **contracts**, not
   manual config.
4. A user can click an actuator command and see its full **causal + authority + safety
   lineage** (`explain`), including the responsible safety rule.
5. Lockstep, branch, and counterfactual **replay** work from a recording.
6. Performance targets in §5 are met and published as a reproducible benchmark suite.
7. The complete reference AUV mission (§38.3) is debuggable and replayable in Studio
   with **no Foxglove dependency**.
8. Live commanding (when enabled) is Ground-authenticated, confirmed, and audited
   (§32.3); disabled by default.
9. A recording or live scene can be inspected immersively on at least one **XR
   headset** via WebXR **and** one via native OpenXR, read-only, at a sustained comfort
   frame rate, degrading gracefully to flat 3D when no headset is present.

---

# 12. Immediate next actions

1. Add the `studio/` workspace members (`engine`, `frontend`, `backend`, `desktop`,
   `benchmarks`) — but only scaffold `engine` + `frontend` now.
2. **S0 spike:** one wgpu scene (GPU plot + point cloud) rendering identically in a
   browser (WASM) and a Tauri window from the same `studio-engine` crate. This proves
   the whole architecture in one shot.
3. **S0 spike:** lazy/indexed open of a large MCAP via the `record` crate, first frame
   in seconds. Defines the data-access layer.
4. Stand up the Foxglove-compat WebSocket in `studio-backend` so live inspection works
   against the Phase-1 runtime immediately (and validates interop).
5. Write **RFC-0006 Studio engine & semantics integration** defining the
   `studio-engine` ↔ core-crates boundary and the wasm-bindgen interface, so the
   semantics-reuse rule is locked before UI work scales.
6. **S0 XR spike (low cost, high signal):** render the existing wgpu point-cloud scene
   in stereo to one WebXR headset and one native OpenXR headset, proving the engine's
   renderer extends to XR before any XR-specific UI is built.
7. Keep Foxglove as the documented stopgap until Studio S3 parity.
