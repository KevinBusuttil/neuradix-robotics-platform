# Neuradix Robotics Platform

Neuradix is a Rust-first, contract-driven robotics development and operations platform: design, program, control, simulate, test, deploy and operate systems across compatible hardware, from tiny Arduino devices to enterprise infrastructure. Classical control, automation, autonomy and optional AI share one engineering model.

The target architecture uses Tiny, MCU, Edge, Workstation and Enterprise execution profiles, with shared contracts, identities, clocks and diagnostics. Profile-specific code generation, executors and device bindings adapt to hardware capabilities. This is the product direction; current implementation remains a foundation prototype.

![Neuradix Robotics Platform ecosystem mind map.](docs/assets/neuradix_platform_ecosystem_mind_map_light.svg)

## Current documentation

- [Documentation index and precedence](docs/README.md)
- [Product, Functional and Technical Specification v0.6](docs/Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) — current functional and technical planning baseline
- [Detailed Implementation Plan v0.4](docs/Neuradix_Implementation_Plan_v0.4.md) — 38 work packages, dependencies, estimates and acceptance evidence
- [Review and Strategy v1.0](docs/Neuradix_Robotics_Platform_Review_and_Strategy_v1.0.md)
- [Current capability and evidence status](docs/Neuradix_Capability_Status.md)
- [A05 bounded Python worker I/O and cleanup](docs/implementation/WP-A05-Bounded-Worker-IO.md)
- [A04.3 host/MCU slew alignment and conformance](docs/implementation/WP-A04.3-Slew-Alignment.md)
- [A04.2 command freshness, generations and API migration](docs/implementation/WP-A04.2-Command-Freshness.md)
- [A04.1 trusted evaluation, numeric validation and API migration](docs/implementation/WP-A04.1-Trusted-Evaluation.md)
- [Gate A codec implementation and validation](docs/implementation/Gate-A-Embedded-Wire-and-ABI.md)
- [Embedded Plan v0.2](docs/Neuradix_Embedded_Profile_Implementation_Plan_v0.2.md), [Studio Plan v0.2](docs/Neuradix_Studio_Implementation_Plan_v0.2.md), [CLI Specification v0.2](docs/Neuradix_CLI_Command_Specification_v0.2.md)
- [RFC Backlog v0.4](docs/Neuradix_RFC_Backlog_v0.4.md), [architecture RFCs](docs/rfcs/), [decision records](docs/decisions/)

## Implementation status

Main now includes the six development increments and the first Gate A codec fixes,
integrated through [PR #6](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/6) and [PR #7](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/7).
The workspace remains an experimental foundation: **16 library/tool crates and
four executable examples**, version 0.0.1.

Implemented foundations include scalar contracts and semantic identity, tagged
clocks, bounded local streams, lifecycle/lockstep processing, graph validation,
scalar authority/lineage/FDIR and supervised Python workers. The integrated work
adds the one-dimensional AUV simulation fixture, experimental MCAP recording,
headless Studio inspection, no_std embedded primitives and serial framing.

Embedded Rust/C++ generation uses canonical field order, the versioned
`neuradix.scalar-le.v2` codec, full wire identities and JSON manifests. Decoders
require the producer's bound wire identity. `--cpp-target avr-uno` rejects
binary64, and generated C++ checks the actual compiler ABI.

Integrated codec baseline validation passed: **191 workspace tests plus two AVR checks**, formatting,
Clippy and the documentation build. The actual ATmega328P compiler built/linked
supported scalar code and rejected binary64 with the intended diagnostic.
See [the implementation evidence](docs/implementation/Gate-A-Embedded-Wire-and-ABI.md).

Current limits:

- `replay run` verifies recorded-data integrity; the runtime lockstep test separately re-executes a processor. An arbitrary changed deployment graph has no CLI runner yet.
- Graph validation does not launch a supervisor or prove physical actuator enforcement; deployment identity still omits resolved behavior.
- A04.1/A04.2/A04.3 are integrated through PR #8/#9/#10. Trusted durable startup, a shared reference clock and periodic evaluation are required; physical safe response and board timing/resource evidence remain open.
- This A05 branch implements bounded Linux Python-worker stdio and cleanup. Heartbeat health, comprehensive resource enforcement, other OS backends and deployment supervision remain open; process separation is not a security sandbox.
- MCAP is a private subset. Serial framing does not negotiate wire identity; recording migration and compact-ID collision enforcement remain open.
- AVR compile/link evidence is not physical board execution. Complete Arduino/MCU firmware, flash/monitor and measured stack/timing remain planned.
- Graphical Studio, general simulator integration, networking/shared memory, ROS/MAVLink bridges, worker clusters and fleet/AI/XR integrations remain planned.

[Capability Status](docs/Neuradix_Capability_Status.md) distinguishes implemented
codec fixes from the remaining acceptance work. Gate A is still open.

## Prerequisites

- Rust toolchain **1.94.1** (pinned in `rust-toolchain.toml`; `rustup` installs it
  automatically). Edition 2024.
- A host C++ compiler (`g++`, `c++` or `clang++`) is required by the code-generation conformance tests.
- `python3` is required for complete CI/worker validation and the Python example. Local worker tests can still skip when it is absent; that is incomplete validation.
- `gcc-avr`, `avr-libc` and `binutils-avr` are required for the separate ATmega328P conformance gate.

## Build and test

```bash
cargo build --locked --workspace
cargo test --locked --workspace
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo doc --locked --workspace --no-deps

# Explicit AVR gate (compile/link and ABI rejection; requires AVR tools):
cargo test --locked -p neuradix-embedded-codegen --test avr -- --ignored --nocapture
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

# Generate a no_std Rust payload and its wire manifest:
cargo run -p neuradix-cli -- contract generate contracts/standard/navigation/vehicle-depth.yaml \
    --language nostd-rust --out-dir target/nrx-generated

# Generate an Uno-compatible scalar conformance fixture (not complete firmware):
cargo run -p neuradix-cli -- contract generate crates/embedded-codegen/tests/fixtures/tiny-telemetry.yaml \
    --language cpp --cpp-target avr-uno --out-dir target/tiny-codegen

# Environment diagnostics:
cargo run -p neuradix-cli -- doctor

# The example writes recordings to temp files; inspect, replay and explain them:
cargo run -p neuradix-example-minimal-depth-stream   # prints the .nrec paths + digest
cargo run -p neuradix-cli -- record inspect /tmp/neuradix-depth-mission.nrec
cargo run -p neuradix-cli -- replay run /tmp/neuradix-depth-mission.nrec --expect-digest <sha256:...>

# Experimental MCAP export; broad external interchange remains unqualified:
cargo run -p neuradix-cli -- record export /tmp/neuradix-depth-mission.nrec --out /tmp/mission.mcap

# Headless Studio inspection:
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

# Deterministic one-dimensional closed-loop AUV fixture:
cargo run -p neuradix-example-auv-depth-sim

# Embedded gate/watchdog/serial behavior exercised on the host:
cargo run -p neuradix-example-embedded-propulsion
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
  record/           # neuradix-record: deterministic recording + replay digest
  command-core/     # no_std shared command validity and physical slew arithmetic
  safety/           # neuradix-safety: authority, constraints, decisions, FDIR
  python/           # neuradix-python: isolated Python worker supervision
  graph/            # neuradix-graph: offline deployment topology + policy compiler
  sim/              # neuradix-sim: fixed-step AUV depth fixture
  studio/           # neuradix-studio: headless recording inspection
  embedded-core/    # no_std identity, health, authority and watchdog primitives
  embedded-transport/ # no_std serial framing, CRC and command metadata binding
  embedded-codegen/ # canonical scalar Rust/C++ codec and target ABI checks
  cli/              # neuradix-cli: the `neuradix` binary
  testkit/          # neuradix-testkit: reusable test utilities
python/             # neuradix_worker.py: the Python-side worker library
contracts/standard/ # authored standard contracts (navigation, perception, control, actuation)
examples/           # minimal-depth-stream, python-worker, auv-depth-sim, embedded-propulsion; reference-auv manifest
docs/rfcs/          # architecture RFCs
docs/decisions/     # architecture decision records (ADRs)
```

## Current implementation priority

Follow [Gates A–E](docs/Neuradix_Implementation_Plan_v0.4.md#3-gates-dependencies-and-effort): trustworthy foundations; one compiled project across actual Uno/MCU and Edge; integrated simulation/Studio/HIL; distributed workers; then a qualified supported workflow. Preserve the AUV fixture as a fast regression. Domain, enterprise HA, broader fleet, AI, Swarm, XR and Flight packs have separate Gate F milestones.

Current commands above run against main. New project/build/flash/simulation/job operations described in the CLI specification are proposed until the capability register and release evidence say otherwise.
