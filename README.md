# Neuradix Robotics Platform

Neuradix is a Rust-first, contract-driven platform for dependable autonomous robots across marine, aerial, ground, embedded and space domains.

![Neuradix Robotics Platform ecosystem mind map.](docs/assets/neuradix_platform_ecosystem_mind_map_light.svg)

## Current architecture documents

- [Product, Functional and Technical Specification v0.5](docs/Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) — authoritative
- [Platform Implementation Plan v0.3](docs/Neuradix_Implementation_Plan_v0.3.md)
- [Studio XR Implementation Plan v0.2](docs/Neuradix_Studio_XR_Implementation_Plan_v0.2.md)
- [Embedded Profile Implementation Plan v0.1](docs/Neuradix_Embedded_Profile_Implementation_Plan_v0.1.md)
- [CLI Command Specification v0.1](docs/Neuradix_CLI_Command_Specification_v0.1.md)
- [RFC Backlog v0.3](docs/Neuradix_RFC_Backlog_v0.3.md)
- Architecture RFCs: [docs/rfcs/](docs/rfcs) · Decision records: [docs/decisions/](docs/decisions)

## Implementation status — foundation increment 1

This repository currently implements the **foundation increments**: the
contract-to-runtime path and a deterministic recording/replay nucleus, proven
end to end on a single in-process stream.

```text
authored contract → parsed & validated contract → deterministic schema identity
→ generated Rust type → typed timestamp & clock domain → bounded in-process stream
→ minimal component lifecycle → deterministic executor → recording
→ lockstep replay reproducing identical control decisions
→ safety authority + constraint gate producing auditable decisions
→ FDIR fault-mode state machine (nominal → degraded → safe → return-to-service)
→ recorded command lineage → `explain` the causal chain of any command
→ offline deployment-graph validation (contracts before connectivity)
→ a deterministic closed-loop simulation (control → safety → simulated plant)
→ MCAP export with cross-container replay equivalence (Foxglove / ROS 2 interop)
→ a headless Studio read model (timeline, channel stats, scalar series)
→ CLI validation, inspection, replay, explain, graph validate and studio → tests
```

### What works today

- **Contracts** (`neuradix-contracts`): YAML `StreamContract` parsing; total
  validation with typed, all-at-once error reporting; a deterministic,
  content-addressed schema identity (`sha256:…`) that is independent of formatting
  and field order; and a deterministic, `rustfmt`-clean Rust code generator.
- **Time** (`neuradix-time`): clock domains, domain-tagged timestamps (cross-domain
  arithmetic is a typed error), a signed nanosecond duration, and an injectable
  `Clock` with a deterministic `ManualClock` (no sleeping, no ambient time).
- **Transport API** (`neuradix-transport-api`): a transport-neutral bounded stream
  with `reject` / `drop-oldest` / `drop-newest` / `keep-latest` overflow policies
  and observable statistics. No backend type leaks into the public API.
- **Runtime** (`neuradix-runtime`): stable component identity, a validated and
  audited lifecycle state machine, execution-class and health models, a minimal
  component manifest and trait, and a **deterministic input-driven executor**
  (`Processor` + `run_lockstep`) that drives component logic under an injected,
  controllable clock — so a recorded run replays to identical outputs.
- **Record** (`neuradix-record`): a native, self-describing, deterministic
  recording container (manifest + channels + schema identities + provenance), a
  payload-agnostic codec, and a `sha256:` replay digest for replay-equivalence
  checks. Parsing is bounds-checked and panic-free. A second, **MCAP** backend
  (Foxglove / ROS 2 interop) sits behind the same `Recording` surface: a native
  recording exported to MCAP **replays to the identical digest** — cross-container
  replay equivalence.
- **Safety** (`neuradix-safety`): the authority + constraint path every actuator
  command traverses — time-bounded authority leases (with permitted envelopes),
  range/slew constraints that name the rule they enforce, fail-safe rejection,
  and an auditable, deterministic `SafetyDecision`. The gate is a `Processor`, so
  safety decisions replay identically. A self-describing `CommandLineage` links
  each command's sensor input → request → authority/constraint outcome → applied
  value for later explanation. An `FdirMonitor` drives an explicit fault-mode
  state machine (nominal → degraded → safe) with confirmation debounce, a
  restart-storm budget and operator return-to-service.
- **Python** (`neuradix-python` + `python/neuradix_worker.py`): run a Python
  component as an **isolated OS process** supervised from Rust over a
  line-delimited JSON protocol, with health, request timeouts, and **crash
  isolation** — a Python crash surfaces as a recoverable error, never a runtime
  crash (v1.0 acceptance §41.6). A bounded restart budget prevents restart
  storms, and a worker's health composes with the FDIR monitor.
- **Graph** (`neuradix-graph`): the deployment compiler that enforces "contracts
  before connectivity". It validates a declarative `RobotDeployment` manifest
  **offline** — node placement, contract provides/requires wiring, acyclicity,
  Python kept off the deterministic control path (EXEC-007), and the rule that an
  actuator may only be commanded through the Safety authority (§16.1) — reporting
  every problem in one pass with stable issue codes, plus a content-addressed
  **deployment identity** for production pinning. Given a contract registry
  (`--contracts <dir>`) it also **resolves every wired contract reference** to a
  real, validated schema and pins the `sha256:` schema identity it resolved to.
- **Sim** (`neuradix-sim`): a deterministic vehicle simulation that closes the
  control loop. A fixed-step vertical-depth **plant**, a **sensor** model and a
  closed-loop **driver** step sensor → controller → plant under an injected
  clock, so the vertical slice runs against a *simulated vehicle* rather than a
  canned input sequence — and two identical runs produce a byte-identical
  trajectory. The controller seam is narrow enough that the crate depends only on
  `neuradix-time`, so a real (or safety-gated) control law drives it without
  coupling the model to the runtime or safety crates.
- **Studio** (`neuradix-studio`): the headless **read model** a Studio/XR UI
  queries — built over the same `Recording` surface, so it works on native or
  MCAP. It answers a **timeline** (per-clock-domain spans + per-channel
  statistics: counts, first/last time, effective rate, payload sizes), **windowed**
  and **nearest-record** queries, and plottable **scalar series** for a chosen
  field (via a caller-supplied decoder, so it commits to no payload encoding).
  Pure and deterministic; overflow-safe even at the extremes of the time domain.
- **CLI** (`neuradix`): `version`, `doctor`, `contract validate|inspect|hash|
  generate`, `record inspect|export` (export to MCAP), `replay run` (with
  `--expect-digest`), `explain command` (reconstruct a command's causal chain from
  a recording), `graph validate` (deployment topology + policy, exit code 10 on
  failure), and `studio timeline|series` (headless inspection), with
  `--output table|json|yaml`, a versioned result envelope and a stable exit-code
  contract (including exit code 9 on a replay-digest mismatch). `inspect`,
  `replay`, `explain` and `studio` accept native `.nrec` or MCAP transparently.
- **Testkit** (`neuradix-testkit`): reusable test utilities (clocks, golden files,
  schema hashing, lifecycle, streams, CLI output) and a dependency-boundary check.
- **Example** (`minimal-depth-stream`): a `VehicleDepth` producer → bounded stream
  → consumer, driven through lifecycle states with a deterministic clock, then
  **recorded and replayed with a verified fidelity check**, a depth controller
  whose **control decisions replay identically** from the recording, and finally
  those commands routed through the **safety gate** (clamped by a range
  constraint, then rejected to a fail-safe output once the authority lease
  lapses), the resulting **command lineage recorded for `explain`**, and an
  **FDIR** health sequence driving nominal → degraded → safe → reset.

### Not yet implemented

The following are **planned and intentionally absent**: Swarm, Aero, Flight,
Ground, Fleet; the Studio and Studio XR **UI** (the headless inspection read
model exists — the graphical/3-D front-end does not); network transport
(Zenoh/DDS) and shared memory; live `record start/stop` against a running graph;
in-process PyO3/Maturin bindings and NumPy zero-copy views (isolated Python
*worker processes* exist); ROS 2 / MAVLink bridges; independent safety-island
deployment (the authority + constraint gate and command-lineage `explain`
exist); and any physical MCU firmware (ESP32/RP2040/STM32) or Arduino
compilation. Public boundaries are designed so these can be added without
exposing backend-specific types.

## Prerequisites

- Rust toolchain **1.94.1** (pinned in `rust-toolchain.toml`; `rustup` installs it
  automatically). Edition 2024.
- `python3` (optional) — only for the `neuradix-python` worker tests and the
  `python-worker` example; those tests skip cleanly when it is absent.

## Build and test

```bash
cargo build --workspace
cargo test  --workspace
cargo fmt   --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo doc   --workspace --no-deps
```

## CLI examples

```bash
# Validate a contract (exit code 3 on failure); machine-readable output:
cargo run -p neuradix-cli -- --output json contract validate contracts/standard/navigation/vehicle-depth.yaml

# Print the content-addressed schema identity:
cargo run -p neuradix-cli -- contract hash contracts/standard/navigation/vehicle-depth.yaml

# Inspect the parsed contract:
cargo run -p neuradix-cli -- contract inspect contracts/standard/navigation/vehicle-depth.yaml

# Generate the Rust projection:
cargo run -p neuradix-cli -- contract generate contracts/standard/navigation/vehicle-depth.yaml \
    --language rust --out-dir /tmp/nrx-generated

# Environment diagnostics:
cargo run -p neuradix-cli -- doctor

# The example writes recordings to temp files; inspect, replay and explain them:
cargo run -p neuradix-example-minimal-depth-stream   # prints the .nrec paths + digest
cargo run -p neuradix-cli -- record inspect /tmp/neuradix-depth-mission.nrec
cargo run -p neuradix-cli -- replay run /tmp/neuradix-depth-mission.nrec --expect-digest <sha256:...>

# Export a recording to MCAP (Foxglove / ROS 2). The MCAP replays to the SAME
# digest, and `inspect`/`replay` accept the .mcap transparently:
cargo run -p neuradix-cli -- record export /tmp/neuradix-depth-mission.nrec --out /tmp/mission.mcap
cargo run -p neuradix-cli -- replay run /tmp/mission.mcap --expect-digest <sha256:...>

# Headless Studio inspection: a recording's timeline (per-domain spans + channel
# rates) and a plottable scalar series from the command-lineage channel:
cargo run -p neuradix-cli -- studio timeline /tmp/neuradix-depth-mission.nrec
cargo run -p neuradix-cli -- studio series /tmp/neuradix-depth-lineage.nrec --field applied

# Explain the causal chain (sensor -> control -> authority/constraints -> applied)
# of the command nearest a given time:
cargo run -p neuradix-cli -- explain command /tmp/neuradix-depth-lineage.nrec --at 450000000

# Validate a deployment manifest offline (exit code 10 on failure); reports the
# content-addressed deployment identity and every topology/policy issue:
cargo run -p neuradix-cli -- graph validate examples/reference-auv/deployment.yaml

# ...and resolve every wired contract reference to a real, registered schema
# (reports the sha256: schema identity each reference pins):
cargo run -p neuradix-cli -- graph validate examples/reference-auv/deployment.yaml \
    --contracts contracts/standard
```

## Examples

```bash
# The full deterministic vertical slice (contract -> stream -> control -> safety
# -> record -> replay -> explain -> FDIR):
cargo run -p neuradix-example-minimal-depth-stream

# An isolated Python worker: detection, a Python crash that is isolated and
# drives FDIR to a safe mode, then a supervised restart (requires python3):
cargo run -p neuradix-example-python-worker

# A closed-loop AUV depth mission: a proportional controller drives a simulated
# plant, but only through the safety gate; converges to the setpoint and proves
# the whole mission is byte-identical on re-run:
cargo run -p neuradix-example-auv-depth-sim
```

It loads the authored contract, derives the stream's capacity and overflow policy
from it, moves a producer and consumer through valid lifecycle states, stamps each
sample with a domain-tagged timestamp from a deterministic clock, exercises the
bounded-stream overflow policy, and prints stream statistics before terminating
deterministically.

## Repository layout

```text
crates/
  contracts/        # neuradix-contracts: model, validation, identity, codegen
  time/             # neuradix-time: clock domains, timestamps, clocks
  transport-api/    # neuradix-transport-api: bounded stream, backend-neutral
  runtime/          # neuradix-runtime: component + lifecycle + deterministic executor
  record/           # neuradix-record: deterministic recording + replay digest (native + MCAP)
  safety/           # neuradix-safety: authority, constraints, decisions, FDIR
  python/           # neuradix-python: isolated Python worker supervision
  graph/            # neuradix-graph: offline deployment topology + policy compiler
  sim/              # neuradix-sim: deterministic depth plant, sensor, closed-loop driver
  studio/           # neuradix-studio: headless inspection (timeline, stats, series)
  cli/              # neuradix-cli: the `neuradix` binary
  testkit/          # neuradix-testkit: reusable test utilities
python/             # neuradix_worker.py: the Python-side worker library
contracts/standard/ # authored standard contracts (navigation, perception, control, actuation)
examples/           # minimal-depth-stream, python-worker, auv-depth-sim, reference-auv (manifest)
docs/rfcs/          # architecture RFCs
docs/decisions/     # architecture decision records (ADRs)
```

## Current implementation priority

The first target is one complete AUV vertical slice:

```text
contracts → Rust/Python APIs → runtime → simulation → safety
→ record/replay → explain → Studio inspection
```

A narrow embedded extension then proves the same contracts on host simulation, one native Rust MCU and one generated Arduino C++ endpoint.
