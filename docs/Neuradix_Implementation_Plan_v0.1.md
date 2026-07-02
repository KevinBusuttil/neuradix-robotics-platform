---
title: "Neuradix Robotics Platform — Implementation Plan"
subtitle: "Engineering execution plan derived from the Functional Specification v0.2"
author: "Engineering"
date: "22 June 2026"
version: "0.1 Draft"
status: "For review"
---

# 0. How to read this document

This is an **engineering implementation plan**, not a restatement of the
[Functional Specification v0.2](./Neuradix_Robotics_Platform_Functional_Specification_v0.2.md).
The spec defines *what* Neuradix must be and *why*. This document defines *how* and
*in what order* a team should actually build it, what to build versus buy, where the
real technical risk sits, and what to deliberately defer.

Where this plan disagrees with, or adds caution to, the spec, those points are
called out explicitly under **Engineering note**. The spec is excellent and
internally consistent; the main risk it carries is **scope**, and most of the
engineering judgement below is about controlling that scope.

Requirement IDs (e.g. `NRX-EDG-001`, `COMP-001`) refer back to the spec.

---

# 1. Executive engineering assessment

## 1.1 The headline

The specification describes **six products that each have a credible standalone
market**:

1. A contract-driven robotics middleware/runtime (competes with ROS 2 / Zenoh-based stacks).
2. A deterministic recording + replay system (competes with Foxglove + custom replay).
3. A physics/marine + space simulator (competes with Gazebo / Basilisk / custom).
4. A flight-software framework with qualification evidence (competes with cFS / F´).
5. Ground + fleet operations (competes with YAMCS / OpenMCT / custom).
6. An engineering Studio GUI (competes with Foxglove Studio / rerun).

Each of items 1, 3, 4, and 5 is, on its own, multiple engineer-years of work.
**Delivering all of them at the quality the spec demands is a 5–8 year, well-funded,
multi-team programme.** The spec's own roadmap (§39) and risk table (§42) already
acknowledge this — the most important risk listed is *"Attempting to match the ROS
ecosystem too early → scope collapse."* This plan takes that warning literally.

## 1.2 The strategy that makes this tractable

**Build one thin, complete, *vertical* slice end-to-end before widening anything.**

The reference AUV (§38) is the correct forcing function. If a single AUV depth-hold
mission can run in simulation and on hardware, be recorded, be replayed
deterministically, have one actuator command *explained*, and survive a Python
crash — using the *same contracts* throughout — then the platform's thesis is proven
and every later capability is an extension of working machinery rather than a leap of
faith.

Everything in this plan is sequenced to reach that vertical slice (≈ spec Phase 0 → 3)
with the **flight/space, fleet, and rich-Studio work explicitly deferred** to after
the core is load-bearing.

## 1.3 The four keystones (get these right or nothing else matters)

These are the components where a wrong early decision is expensive or impossible to
undo later. They deserve disproportionate senior attention and Phase 0 prototyping:

| # | Keystone | Why it's irreversible | Spec ref |
|---|----------|----------------------|----------|
| K1 | **Contract system + codegen** (IDL, units, frames, clock domains, versioning) | Every component, every wire format, every binding, and all tooling depends on it. Changing the contract model later breaks the entire ecosystem. | §10, §15 |
| K2 | **Transport-neutral data plane** (in-proc / shm / network behind one API) | If the abstraction leaks (e.g. Zenoh semantics bleed into the public API), the platform is permanently coupled to a backend. Zero-copy + lease ownership is the hard part. | §9, §11, §13 |
| K3 | **Time + determinism model** (injected clocks, seeds, no ambient `now()`) | Deterministic replay (the headline differentiator) is only achievable if *nothing* reads wall-clock or RNG ambiently. This is architectural, not a feature you add later. | §14, §24 |
| K4 | **Component model + lifecycle + execution classes** | The supervision, safety, and scheduling guarantees all hang off this contract between component and runtime. | §8, §12 |

**Engineering note:** The spec lists Studio, Fleet, and Flight prominently, which
can create pressure to start them early. Resist this. K1–K4 are the load-bearing
walls; the rest are rooms you can add later.

## 1.4 Build vs. buy posture (strongly endorse the spec's §3.9 / §43)

The spec's technology choices are sound and should be locked in. Do **not** reinvent
any of these:

| Concern | Use | Do NOT build |
|---|---|---|
| Network transport | **Zenoh** (behind our API) | A new pub/sub protocol |
| Recording container | **MCAP** | A new log format |
| Observability | **OpenTelemetry + `tracing`** | A new metrics/trace stack |
| Python bindings | **PyO3 + Maturin** | A custom FFI bridge |
| Packaging | **OCI artifacts** (e.g. `oras`) | A new registry/format |
| Serialization | **Protobuf** (wire) + YAML/TOML (authoring) | A new binary serializer |
| Embedded executors | **RTIC + Embassy** | A new RTOS |
| Early visualization | **Foxglove Studio** (reads MCAP) — *interim stopgap only* | A bespoke GUI in Phase 1 |
| Flight reference patterns | study **cFS / F´**, **RTEMS** | a flight OS |
| Qualified toolchain | **Ferrocene** (when flight is real) | a qualified compiler |

This list converts roughly half the spec's surface area from "invent" to "integrate."

---

# 2. Guiding implementation principles

These translate the spec's design principles (§3) into rules the team applies daily.

1. **Vertical before horizontal.** A feature is not "done" until it works from
   contract → SDK → runtime → transport → record → replay → Studio inspection for at
   least one real signal. Avoid building broad layers that don't connect.
2. **Determinism is a constraint, enforced by lints/CI, not a hope.** No component
   may call `std::time::SystemTime::now`, spawn unmanaged threads, or use ambient
   RNG. The deterministic-profile clippy lint set (§18.3) is written in Phase 0 and
   gates CI from then on.
3. **The public API is backend-neutral and reviewed as such.** Any PR that exposes a
   Zenoh, Tokio, or PyO3 type across the `neuradix-sdk` / `neuradix-contracts`
   boundary is rejected. (Add an automated API-surface snapshot test.)
4. **Bounded by default (§3.2).** Every queue, pool, and buffer has a declared
   capacity and overflow policy in code review checklists.
5. **Evidence is a build output, not a document sprint (§3.11).** SBOM, schema
   hashes, and manifest hashes are emitted by the build pipeline from day 1, even
   when nothing consumes them yet — cheap now, very expensive to retrofit.
6. **Two reference users drive every API:** the Rust component dev and the Python AI
   dev. If the ergonomics in §18.2 / §19.3 don't hold, the API is wrong.
7. **Profiles restrict; they do not fork.** Edge/Embedded/Flight share one contract
   language and codegen. A profile is a *policy + feature-flag + lint* configuration
   over shared crates, never a parallel codebase (§7.1, §3.12).

---

# 3. Repository & workspace bootstrap (Week 1)

Adopt the monorepo layout from spec §36 with a Cargo workspace, but **create crates
lazily** as phases need them rather than scaffolding 25 empty crates up front.

## 3.1 Initial workspace skeleton

```text
neuradix/
  Cargo.toml                 # workspace
  rust-toolchain.toml        # pinned toolchain (reproducibility from day 1)
  deny.toml                  # cargo-deny: licenses + advisories
  .github/workflows/         # CI: build, test, clippy, fmt, deny, miri (gated)
  crates/
    contracts/               # K1 — contract model + codegen (FIRST)
    runtime/                 # K4 — lifecycle, supervision, executors
    data-plane/              # K2 — primitives over transport-api
    transport-api/           # K2 — the neutral trait surface
    transport-local/         # in-process + local IPC
    transport-shm/           # shared memory + leases
    time/                    # K3 — clock domains, injected clocks
    cli/                     # neuradix CLI (thin wrapper, grows over time)
    testkit/                 # deterministic test harness
  python/                    # PyO3/Maturin package (Phase 1)
  contracts/standard/        # canonical contract definitions (data, not code)
  examples/minimal-robot/
  rfcs/                      # RFC-0001..0005 land here in Weeks 1-2
  docs/
  governance/                # CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, LICENSE
```

**Engineering note — crate layering:** Enforce an acyclic dependency layering and
encode it in CI (e.g. `cargo-deny` bans, or a small dependency-graph test):

```
contracts  →  (no internal deps; pure model + codegen)
time       →  contracts
transport-api → contracts, time
transport-local / -shm / -zenoh → transport-api
data-plane → transport-api, contracts, time
runtime    → data-plane, contracts, time
cli, testkit, python → runtime, data-plane, contracts
```

`contracts` and `time` must never depend on `runtime` or any transport. This keeps
the contract model usable by codegen, embedded, and flight targets that will never
link the full runtime.

## 3.2 Day-1 governance & hygiene (cheap, high-leverage)

Per spec §37.3: land `LICENSE` (Apache-2.0), `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`,
`SECURITY.md`, and a `governance/ROADMAP.md` that points at this plan. Set up CI
gates (fmt, clippy, test, `cargo-deny`) *before* the first feature PR — retrofitting
green CI onto a large codebase is painful.

---

# 4. Crate-by-crate implementation breakdown

This is the concrete decomposition of the keystones and supporting work. Effort
estimates are **rough order-of-magnitude engineer-weeks for a senior generalist**,
intended for sequencing, not commitment.

## 4.1 `contracts` (K1) — the highest-leverage crate

The keystone. Build it first and review it hardest.

**Scope**
- Contract manifest schema (YAML/TOML authoring) per §10.1–§10.3: payload schema ref,
  semantics (quantity/unit/frame/positive-direction), time (clock domain, max age),
  delivery policy, quality/uncertainty.
- A canonical, content-addressed **schema identifier** (§10.5) — hash of the
  normalized contract. This ID threads through records, evidence, and compatibility.
- Codegen: Rust types first; Python type hints and Protobuf schema next; JSON Schema
  for config/web later (§10.5, §10.2).
- Compatibility engine implementing §10.4 semantic-version rules (patch=wire-compat,
  minor=additive, major=adapter-required) + unit/frame compatibility checks.
- Unit and frame metadata (§15) as first-class contract fields, with compile-time
  Rust unit wrappers where practical and runtime metadata for Python.

**Sequencing within the crate**
1. Manifest parse + validate + content-addressed ID.
2. Rust codegen (one contract: `DepthMeasurement`) → unblocks everything else.
3. Compatibility checker + machine-readable report.
4. Protobuf + Python-hint generation.
5. Conformance-test generation (§34.2) — can trail slightly.

**Effort:** 8–12 wk to a solid v0; it will keep evolving.

**Engineering note:** Decide *now* whether the authored contract format is the source
of truth (recommended) and Protobuf is a *generated* projection, or vice versa.
The spec implies authored YAML → generated Protobuf (§10.2); lock that direction in
**RFC-0002** before writing codegen, because reversing it later is a rewrite.

## 4.2 `time` (K3) — small crate, outsized importance

**Scope**
- Clock-domain enum and typed timestamps (§14.1): monotonic, UTC, sensor-hw,
  synchronized-monotonic, sim, replay, GNSS. A timestamp without a domain must not
  compile/serialize across a boundary in production profiles (§14.1).
- A `Clock` trait injected everywhere; concrete impls: system, simulation (steppable),
  replay (driven from a recording).
- Sync-status surface (§14.3): offset, uncertainty, holdover, step events.

**Effort:** 3–4 wk. Do it early; it's a dependency of the data plane and of
determinism enforcement.

## 4.3 `transport-api` + `transport-local` + `transport-shm` (K2)

**Scope**
- `transport-api`: the neutral trait surface — endpoints, publish/subscribe, query,
  lease/loan semantics for large buffers (§11.3), policy descriptors (§13.4). This is
  the contract that keeps Zenoh out of the public API.
- `transport-local`: in-process typed channels + local IPC for small payloads.
- `transport-shm`: shared-memory pool with **lease-based ownership, crash recovery,
  bounded pools, integrity checks, fallback-to-copy** (§13.3). This is the hard one.

**Engineering note — the riskiest single piece is the loaned/zero-copy buffer with
lease lifetime that also crosses the Python boundary** (§11.3 + §19.1 NumPy zero-copy).
A producer must not reuse memory while consumer leases (including a Python NumPy view
held past the GIL) are live. Prototype this in **Phase 0** with a deliberate
crash/abuse test before committing to the design. If it proves too costly, the
fallback is copy-on-cross-process for v1 and zero-copy only in-process — acceptable
and far safer than a subtly broken lease system.

**Effort:** local 4–6 wk; shm 8–12 wk including crash-recovery and the Python view.

## 4.4 `data-plane` (K2 cont.) — the six primitives

**Scope:** Implement §9's six primitives over `transport-api`:
Stream, State (revisioned, conditional update §9.2), Command (id/idempotency/deadline/
auth-result §9.3), Task (long-running, cancellable §9.4), Event (immutable §9.5),
Query (§9.6). Plus the common envelope (§11.1).

**Sequencing:** Stream + State first (unblock the AUV slice), then Command + Event,
then Task + Query. Bounded queues + overflow policy (§12.4 EXEC-001..006) are built
into the primitives, not bolted on.

**Effort:** 8–10 wk for all six to a usable level.

## 4.5 `runtime` (K4) — lifecycle, supervision, execution

**Scope**
- Component trait + lifecycle state machine (§8.2) and lifecycle requirements
  (COMP-001..007): observable transitions with reason/initiator/timestamp/result,
  restart policy + crash-loop quarantine (COMP-004), stable logical identity (COMP-007).
- Execution classes (§12.1) and executors (§12.2): start with the **deterministic
  executor** (periodic/event-triggered, no shared async pool — §12.2) and the
  **Tokio async executor** for I/O. Fixed-priority / RT-Linux and embedded adapters
  come later.
- Component isolation classes (§8.4): in-process Rust + native process + Python worker
  first; Wasm and embedded later.
- The `#[component]` / `#[execution(...)]` proc-macro ergonomics from §18.2.

**Engineering note:** The `#[component]` proc-macro is developer-facing API surface.
Get the *trait* right first and let people implement it by hand; add the macro once
the shape is stable. A premature macro ossifies a wrong design behind generated code.

**Effort:** 10–14 wk for the deterministic+async core with supervision.

## 4.6 `python` (PyO3/Maturin) — first-class isolated extension

**Scope** (§19): managed Python worker process, generated type hints, async component
API, NumPy zero-copy *read-only* views over shm buffers, supervisor-enforced
CPU/mem/GPU limits + restart isolation (§19.4), locked/content-addressed dependency
envs.

**Engineering note:** **Process isolation is non-negotiable and is the whole point**
(acceptance criterion §41.6: a Python crash must not kill control/safety). Do not
attempt in-process embedded Python for components. PyO3 is used for the *client/runtime
binding* (§19.2), and the Python component runs as a supervised separate process
communicating over the data plane.

**Effort:** 8–10 wk for worker + zero-copy read views + supervision.

## 4.7 `record` + replay — the differentiator

**Scope** (§24): MCAP writer/reader; record streams/state/commands/tasks/events plus
**topology manifest, component hashes, contract schemas, config snapshots, clock-sync
data, seeds** (§24.2, §23.5). Replay clock (driven via `time`'s replay clock).
Replay modes incrementally: real-time → lockstep/step → branch/counterfactual (§24.4).

**Engineering note:** Recording the *environment* (hashes, topology, seeds, clock
relations), not just the data, is what makes replay deterministic and is the thing
teams routinely under-build. Specify the record manifest in **RFC-0005** and capture
the full environment from the very first recording, even before fancy replay modes
exist.

**Effort:** record 6–8 wk; full replay-mode set 8–12 wk (spread across phases).

## 4.8 `safety` — authority, constraints, FDIR

**Scope** (§16): authority manager + time-bounded leases (§16.3), constraint engine
(bounds/rate/slew/geofence/depth — §16.4), command path (§16.2), fault containment
(§16.5), FDIR state model (§16.8). Independent safety-island deployment (§16.7) is a
*deployment topology* enabled by clean interfaces, not new code initially.

**Effort:** 10–14 wk for Edge-grade authority+constraints; FDIR maturity is ongoing.

## 4.9 `graph` / topology compiler — the gatekeeper

**Scope** (§6.3, §28.2): validate contract/schema/unit/frame/clock compatibility,
missing providers, queue bounds, resource capacity, node-arch compatibility,
permissions, actuator-authority path, safety-policy presence, **Python/AI in
deterministic paths** (§12.4 EXEC-007, §19.4), and package signatures. Emits the
content-addressed deployment manifest (§28.4).

**Engineering note:** This compiler is where the "contracts before connectivity"
thesis (§3.1) pays off — it's high value and should appear (minimally) early so that
"invalid systems are rejected before runtime." Start with offline static validation
of a YAML deployment; add live/placement logic later.

**Effort:** 6–8 wk for static validation v0; grows continuously.

## 4.10 `cli` + `testkit` (continuous)

`cli` (§33) starts as a thin dispatcher and grows command-by-command alongside each
capability (`contract check`, `build`, `graph validate`, `sim run`, `record`, `replay`,
`explain`, `deploy`, `doctor`). Human + JSON output from the start (§33). `testkit`
provides the deterministic harness, fault-injection hooks (§34.3), and replayable CI
failure artifacts (§34.4) — built alongside, used by every other crate's tests.

---

# 5. Phased delivery plan

This maps the spec's roadmap (§39) and 90-day plan (§44) to concrete engineering
milestones with **explicit exit criteria**. Calendar durations assume a small senior
team (see §8); they are sequencing estimates, not commitments.

## Phase 0 — Architecture validation & de-risking (≈ 8–12 weeks)

**Goal:** prove the four keystones are buildable before committing to breadth. This
phase is about *answering questions*, not shipping features.

**Deliverables**
- RFC-0001 Component model, RFC-0002 Contract model, RFC-0003 Execution classes,
  RFC-0004 Transport abstraction, RFC-0005 Record/Replay model (§44).
- Working spikes (throwaway-quality acceptable) proving:
  - K1: one contract → generated Rust type, with content-addressed ID.
  - K2: the *same* `StreamWriter`/`StreamReader` API working over in-process **and**
    Zenoh **and** shared memory, with no backend type in the public signature.
  - K2 hard case: a shm zero-copy buffer leased to a Python NumPy view, with a test
    that proves the producer cannot corrupt a live lease (and a clean crash-recovery).
  - K3: a component that produces identical output across two runs with an injected
    clock + fixed seed (determinism baseline).
- Performance baselines for in-proc small messages and shm large buffers (§35.2).

**Exit criteria (go/no-go gate):** all four spikes pass; the transport-neutral API
shape is ratified in RFC-0004; the zero-copy/lease approach is decided (full or
copy-on-cross-process fallback). **If the lease/zero-copy spike fails, descope to
copy-on-cross-process for v1 — do not let this block the programme.**

**Engineering note:** Phase 0 is the most important phase and the one most often
skipped under delivery pressure. Treat the spikes as disposable; the *answers* and
the RFCs are the real output.

## Phase 1 — Minimum viable platform (≈ 4–6 months)

**Goal:** a usable single-robot platform — the spec's Phase 1 (§39).

**Deliverables**
- Productionized `contracts` (Rust + Python hints + Protobuf), `time`,
  `transport-api/-local/-shm/-zenoh`, `data-plane` (Stream/State/Command/Event first),
  `runtime` (deterministic + async executors, lifecycle, supervision).
- Python worker SDK (PyO3/Maturin) with process isolation + read-only zero-copy.
- `graph` static validation v0; `record` to MCAP with full environment capture.
- CLI: `new`, `contract check`, `build`, `test`, `graph validate`, `record start`.
- **Minimal Studio = Foxglove + MCAP** (do not build a custom GUI yet — §42 risk).
- Vertical demo (§44 weeks 6–8): Rust camera/sensor producer → Python detector
  consumer over shm, recorded to MCAP, inspected in Foxglove.

**Exit criteria:** the §44 weeks-6–8 and 9–10 demos pass; a Python process crash is
demonstrably isolated from the runtime; a fixed-seed component replays identically.

## Phase 2 — Dependability (≈ 4–6 months)

**Goal:** the spec's Phase 2 (§39) — make it *trustworthy*.

**Deliverables**
- `safety`: authority leases + constraint engine + fault containment, protecting the
  reference actuators (§41.8).
- Health supervision + FDIR primitives (§16.5, §16.8).
- Security: component identity, signed packages (OCI + signatures), SBOM emission,
  capability permissions (§26).
- Static/immutable production topology + content-addressed manifest (§28.4).
- Deterministic lockstep replay + causal command lineage / `explain` (§25.3–§25.4).
- Task + Query primitives completed.
- First embedded endpoint on one MCU family (RTIC or Embassy) over CAN/serial (§20),
  with a Linux gateway (§20.4).

**Exit criteria:** authority/safety path protects all reference actuators; a single
command can be `explain`-ed to its causal chain; signed deploy + verify works; one MCU
endpoint reports health/watchdog into the graph.

## Phase 3 — Marine reference platform (≈ 6 months) — *the proof point*

**Goal:** the complete AUV vertical slice (§38) — the spec's Phase 3 and the heart of
v1.0 acceptance (§41).

**Deliverables**
- Marine simulation profile (§23.4): 6-DOF dynamics, buoyancy, hydrodynamic damping,
  thruster curves, currents, depth/IMU/DVL/sonar/camera/modem sensor models, energy.
  (Evaluate reusing an existing marine sim core before building from scratch.)
- AUV capability drivers (§22.1): IMU, depth, DVL, sonar, camera, thruster, battery.
- Constrained-link + offline mission operation (§29).
- The full §38.3 demonstration mission, reproducible from public instructions (§41.15),
  including DVL-loss degraded mode, comms-loss policy, log prioritization, replay, and
  a counterfactual controller swap (§38.3 steps 11–12).
- ROS 2 + MAVLink bridges (§31.2 priority 1) — functional, as a boundary, not adopted
  internally (§3.8).
- Published benchmark + evidence data (§35.2).

**Exit criteria = spec §41 (Version 1.0 acceptance), items 1–15.** This is the
milestone at which Neuradix is a real product.

**Engineering note — Studio decision point:** Only *after* Phase 3 stabilizes should
serious investment in a custom Studio (§32) begin, and even then start with the
*inspection* views (graph, signals, lineage, replay) before authoring views — exactly
as §42's risk row advises. Foxglove + the CLI cover most needs until then.

**Decision update (June 2026):** Neuradix Studio is now scoped as a *first-class,
differentiating product* — a contract-native, high-performance, web/desktop **and
XR-headset** observability tool intended to equal and surpass Foxglove. It is still
built **inspection-first, growing alongside the core** (Foxglove remains the interim
stopgap until Studio reaches parity at S3). The full design, architecture (shared
Rust + wgpu engine targeting WASM, native, WebXR and OpenXR), phasing and acceptance
criteria are in
[Neuradix Studio — Implementation Plan](./Neuradix_Studio_Implementation_Plan_v0.1.md).

## Phase 4+ — Space, Flight, Ground, Fleet (post-v1.0; multi-year)

These map to spec Phases 4–6 (§39) and are deliberately **out of the v1.0 critical
path**. Sequencing recommendation:

1. **Ground** (§7.7) command/telemetry/timeline/procedures + dictionaries generated
   from the same contracts (§21.6) — useful even for marine ops.
2. **Space simulation** models (§21.13, §23.6) — extends the existing sim, low risk.
3. **Flight Alpha** (§21, §21.14): restricted static runtime, rate-group scheduler,
   command/telemetry/events/params/watchdog/FDIR, one RTOS/bare-metal target. Adopt
   **Ferrocene** here. Treat as an *engineering prototype*, never a certified product.
4. **Fleet** (§29.3) multi-robot operations.
5. **Registry, Wasm plugins, model registry, conformance portal** (§40 P3).

**Engineering note — flight is a programme, not a phase.** The spec is admirably
honest that installing Neuradix confers no certification (§16.6, §21.1, §7.4
NRX-FLT-008). Hold that line in *every* external communication. Flight should remain
**paper-only architecture review** (§44 weeks 11–12 explicitly recommend a paper
review of the Flight/safety-island profiles) until the core is proven in the field.
Do not let flight ambitions pull senior engineers off the marine slice.

---

# 6. Cross-cutting concerns (wired in from Phase 0, not retrofitted)

| Concern | Built-in from | Mechanism |
|---|---|---|
| **Determinism enforcement** | Phase 0 | clippy lint profiles (§18.3) banning `SystemTime::now`, ambient RNG, unmanaged threads; CI-gated |
| **Backend-neutrality** | Phase 0 | API-surface snapshot test; PR review rule; layering enforced in CI |
| **Bounded resources** | Phase 1 | queue capacity + overflow mandatory in primitives; runtime metrics expose occupancy/drops (§12.4) |
| **Observability** | Phase 1 | OpenTelemetry + `tracing` from the first component (§25.1); mandatory metrics (§25.2) |
| **Evidence/SBOM** | Phase 1 | build pipeline emits SBOM, schema hashes, manifest hashes (§3.11, §16.6) |
| **Security** | Phase 2 (identity scaffolding earlier) | signed OCI artifacts, capability permissions, secret references (§26) |
| **Testing** | Phase 0 | testkit + deterministic harness; CI gates: fmt, clippy, test, `cargo-deny`, Miri (on unsafe), fuzzing corpus (§35.3) |
| **Reproducible CI failures** | Phase 1 | every sim/scenario failure emits a replayable artifact (seed+topology+config+versions+data) (§34.4) |

**Engineering note on `unsafe` and Miri:** The shm/lease and PyO3 code will contain
the bulk of the platform's `unsafe`. Quarantine it in `transport-shm` and the python
binding, document each block with a justification (§21.10 anticipates this for
flight), and run Miri + sanitizers on those crates in CI from the moment they exist.

---

# 7. Technical risk register (engineering additions to spec §42)

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Zero-copy lease lifetime across the Python/GIL boundary proves unsafe or too costly | Med | High | Prototype in Phase 0; **fallback = copy-on-cross-process for v1**, zero-copy in-process only |
| Contract model needs breaking changes after ecosystem exists | Med | High | Heavy Phase 0 RFC review of K1; semantic-version + adapter discipline (§10.4); keep `contracts` dependency-free |
| Backend (Zenoh) semantics leak into public API | Med | High | API-surface snapshot test; strict `transport-api` boundary; code-review gate |
| Determinism quietly broken by an ambient `now()`/thread/RNG | High | High | Lint-enforced from Phase 0; replay-equivalence test in CI |
| Marine simulator under-estimated (fidelity is deep work) | High | Med | Evaluate reusing existing 6-DOF marine cores; treat sim as its own sub-project with its own lead |
| Scope creep into Flight/Fleet/Studio before core is load-bearing | **High** | **High** | This plan's phase gates; flight stays paper-only until post-v1.0; Studio = Foxglove until post-Phase 3 |
| Proc-macro / generated-code ergonomics ossify a wrong API | Med | Med | Stabilize the *trait* before the macro; treat macros as sugar over a hand-implementable API |
| Team too small for parallel sub-platforms | High | High | Single vertical slice; do not staff flight/fleet/studio in parallel with the core |
| "Same binary in sim and hardware" (§3.4) breaks on arch differences | Med | Med | Spec already hedges with SHOULD (§7.6 NRX-SIM-003); enforce via capability interfaces, accept recompiles per target |

---

# 8. Team, resourcing & sequencing reality

**Engineering note — be honest about headcount.** The spec is sized for a funded
team. Mapping the keystones to people:

| Role | Owns | Phase emphasis |
|---|---|---|
| **Tech lead / architect** | RFCs, contract model, API neutrality, cross-crate coherence | All; heaviest in Phase 0 |
| **Senior systems (Rust) #1** | transport-api/-shm, data-plane, zero-copy/leases | 0–2 |
| **Senior systems (Rust) #2** | runtime, executors, lifecycle, supervision, safety | 0–2 |
| **Python/ML integration eng** | PyO3/Maturin SDK, process isolation, zero-copy views | 1+ |
| **Simulation eng** | marine sim, sensor models, scenarios | 3 (ramps in Phase 2) |
| **Tooling/DevEx eng** | CLI, testkit, CI, codegen plumbing, record/replay | 1+ |
| **(later) Embedded eng** | MCU profile, RTIC/Embassy, gateway | 2+ |
| **(later) Flight/assurance eng** | Flight runtime, evidence kit, Ferrocene | post-v1.0 |

A credible **minimum core team is ~4–5 senior engineers** to reach v1.0 (Phase 3) in
roughly **18–24 months**. With 2–3 engineers it is still achievable but the timeline
roughly doubles and Flight/Fleet must be firmly deferred. Attempting the full spec
breadth with a small team is the single most likely way to fail.

---

# 9. Immediate next actions (first two weeks — concrete)

Aligned with spec §44 weeks 1–2, ordered for execution:

1. **Lock foundational decisions** (spec §43 items 1–10): ratify Rust core + isolated
   Python, transport-neutral API + Zenoh, MCAP, OpenTelemetry, OCI, Apache-2.0,
   AUV-first. Record them as ADRs in `docs/adr/`.
2. **Bootstrap the workspace** (§3.1): Cargo workspace, pinned `rust-toolchain.toml`,
   `deny.toml`, CI (fmt/clippy/test/deny), governance files. CI must be green before
   feature work.
3. **Write RFC-0001…RFC-0005** (component, contract, execution, transport, record).
   RFC-0002 (contract) and RFC-0004 (transport) are the critical ones — they decide
   K1 and K2 and must be reviewed hardest.
4. **Define the reference AUV minimum topology** (§44) — the smallest depth-hold graph
   (depth sensor → estimator → depth controller → safety limiter → thruster model)
   that exercises Stream, State, Command, Event, record, and replay. This becomes the
   running fixture for every phase.
5. **Start the four Phase-0 spikes** (§5 Phase 0), especially the shm-zero-copy-to-
   Python lease spike, because its result (full vs. fallback) shapes everything after.

**Definition of done for the first two weeks:** RFCs in review, green CI on an empty
workspace, the AUV minimum-topology contracts authored, and the determinism +
zero-copy spikes underway.

---

# 10. Summary

- The spec is strong; the dominant risk is **scope**, and this plan controls it by
  driving relentlessly to **one complete AUV vertical slice** (Phase 0 → 3 = v1.0).
- **Four keystones** (contracts, transport-neutral data plane, time/determinism,
  component/lifecycle) are irreversible decisions — front-load them with senior
  attention and Phase-0 prototypes, especially the zero-copy/lease + Python boundary.
- **Buy, don't build** the integrable pieces (Zenoh, MCAP, OTel, PyO3, OCI, Foxglove,
  RTIC/Embassy, Ferrocene) — this halves the surface area.
- **Defer flight, fleet, and custom Studio** until the core is proven in the field;
  keep flight paper-only and never overstate certification.
- **Be honest about team size and timeline:** ~4–5 senior engineers, ~18–24 months to
  a real v1.0; less staffing means firmer deferral and a longer clock.

The next concrete step is §9: lock the §43 decisions as ADRs, bootstrap the workspace
with green CI, write RFC-0002/0004 first, and start the Phase-0 zero-copy and
determinism spikes.
