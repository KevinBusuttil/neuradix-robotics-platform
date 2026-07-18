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
→ CLI validation, inspection and replay → automated tests
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
  checks. Parsing is bounds-checked and panic-free. (MCAP is a planned backend
  behind the same interface.)
- **CLI** (`neuradix`): `version`, `doctor`, `contract validate|inspect|hash|
  generate`, `record inspect`, and `replay run` (with `--expect-digest`), with
  `--output table|json|yaml`, a versioned result envelope and a stable exit-code
  contract (including exit code 9 on a replay-digest mismatch).
- **Testkit** (`neuradix-testkit`): reusable test utilities (clocks, golden files,
  schema hashing, lifecycle, streams, CLI output) and a dependency-boundary check.
- **Example** (`minimal-depth-stream`): a `VehicleDepth` producer → bounded stream
  → consumer, driven through lifecycle states with a deterministic clock, then
  **recorded and replayed with a verified fidelity check**, and finally a depth
  controller whose **control decisions are shown to replay identically** from the
  recording (live control == replayed control).

### Not yet implemented

The following are **planned and intentionally absent**: Swarm, Aero, Studio and
Studio XR, Flight, Ground, Fleet; network transport (Zenoh/DDS), shared memory,
MCAP recording containers (only the native container exists so far), live
`record start/stop` against a running graph, Python/PyO3 bindings; ROS 2 /
MAVLink bridges; the safety/authority path and command lineage; and any physical
MCU firmware (ESP32/RP2040/STM32) or Arduino compilation. Public boundaries are
designed so these can be added without exposing backend-specific types.

## Prerequisites

- Rust toolchain **1.94.1** (pinned in `rust-toolchain.toml`; `rustup` installs it
  automatically). Edition 2024.

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

# The example writes a recording to a temp file; inspect and replay it:
cargo run -p neuradix-example-minimal-depth-stream   # prints the .nrec path + digest
cargo run -p neuradix-cli -- record inspect /tmp/neuradix-depth-mission.nrec
cargo run -p neuradix-cli -- replay run /tmp/neuradix-depth-mission.nrec --expect-digest <sha256:...>
```

## Minimal depth-stream example

```bash
cargo run -p neuradix-example-minimal-depth-stream
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
  runtime/          # neuradix-runtime: component + lifecycle model
  record/           # neuradix-record: deterministic recording + replay digest
  cli/              # neuradix-cli: the `neuradix` binary
  testkit/          # neuradix-testkit: reusable test utilities
contracts/standard/ # authored standard contracts (e.g. navigation/vehicle-depth)
examples/           # minimal-depth-stream
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
