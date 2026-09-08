---
title: "Neuradix CLI Command Specification"
author: "Engineering"
date: "8 September 2026"
version: "0.2 Draft"
status: "Current planning baseline; implemented subset identified below"
---

# Version and implementation scope

This v0.2 draft supersedes v0.1 and aligns with [Neuradix_Robotics_Platform_Functional_Specification_v0.6.md](Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) and [Neuradix_Implementation_Plan_v0.4.md](Neuradix_Implementation_Plan_v0.4.md). The inherited command tree is a **target API**, not an implemented-command inventory. See the repository README and [Neuradix_Capability_Status.md](Neuradix_Capability_Status.md) for current commands.

Current `replay run --expect-digest` verifies recorded data integrity; it does not run a changed component graph. Keep that behaviour compatible until an explicit CLI version/migration decision. Proposed program replay uses a separately named test operation below. Preserve existing structured result/exit-code contracts; any extension requires fixtures and a documented compatibility policy.

# Implemented embedded generation

Main includes `contract generate <file> --language rust|nostd-rust|cpp --out-dir <dir>`.
For C++ only, `--cpp-target portable|avr-uno` selects the numeric profile;
the default is `portable`. Selecting it with another language returns exit code 2.
An unsupported Uno binary64 field returns exit code 3 before creating output.

The embedded languages (`nostd-rust`, `cpp`) produce source plus
`<stem>.wire.json` with codec, schema and full wire identity, exact payload length
and ordered fields/offsets. Result data includes `wireId`, `codecId`,
`wireManifest` and `cppTarget` (null where inapplicable). Host `rust` generation
has no embedded codec manifest. The decoder signature now requires the producer's
wire identity; [migration rules](implementation/Gate-A-Embedded-Wire-and-ABI.md#wire-binding-and-migration)
cover upgrading both endpoints and preserving legacy provenance.

Main also includes `record export` for the experimental MCAP subset and
`studio timeline|series` for headless inspection. These commands do not establish
general MCAP interoperability or a graphical Studio. Project build/flash/run and
the broader command tree below remain planned.

# Purpose

This document defines the target command language and automation contract for the `neuradix` CLI; the implemented subset is identified above.

# Command tree

```text
neuradix
├── new
├── init
├── build
├── check
├── contract
├── component
├── graph
├── run
├── stop
├── inspect
├── record
├── replay
├── explain
├── sim
├── test
├── embedded
├── package
├── deploy
├── registry
├── bridge
├── swarm
├── aero
├── ground
├── fleet
├── studio
├── config
├── context
├── auth
├── doctor
├── completion
└── version
```

# Global flags

```text
--context
--profile
--robot
--swarm
--output table|json|yaml|jsonl
--offline
--timeout
--at
--dry-run
--yes
--verbose
--quiet
```

# Result envelope

Machine-readable commands SHOULD return:

```json
{
  "apiVersion": "cli.neuradix.io/v1alpha1",
  "kind": "CommandResult",
  "command": "contract.validate",
  "status": "success",
  "startedAt": "2026-07-17T10:00:00Z",
  "finishedAt": "2026-07-17T10:00:00Z",
  "context": "local",
  "data": {},
  "warnings": [],
  "errors": []
}
```

# Exit codes

| Code | Meaning |
|---:|---|
| 0 | success |
| 1 | general failure |
| 2 | invalid use |
| 3 | contract validation |
| 4 | compatibility |
| 5 | connectivity |
| 6 | authentication |
| 7 | authorization |
| 8 | safety rejection |
| 9 | determinism/replay mismatch |
| 10 | deployment validation |
| 11 | partial operation |
| 12 | timeout |

# MVP commands

```bash
neuradix init
neuradix contract validate
neuradix contract generate
neuradix build
neuradix run
neuradix graph
neuradix component list
neuradix component health
neuradix inspect stream
neuradix record start
neuradix record stop
neuradix replay run
neuradix explain command
neuradix sim run
neuradix test determinism
neuradix doctor
```

# Embedded commands

```bash
neuradix embedded targets
neuradix embedded new <name> --target <target>
neuradix embedded check
neuradix embedded generate
neuradix embedded build
neuradix embedded size
neuradix embedded flash
neuradix embedded monitor
neuradix embedded inspect
neuradix embedded test
neuradix embedded provision
neuradix embedded update
```

# Safety

Live mutation commands must use authenticated authority and must not bypass onboard Safety. Direct actuator development commands require a hardware-test profile, reason, audit and explicit target.


# Unified project and distributed run additions

The following operations are **proposed**, pending RFC-0013 amendment and the implementation work packages. Examples identify command families, not executable instructions for the current build.

| Proposed operation | Responsibility | Gate/package |
|---|---|---|
| `neuradix project validate` | Validate system model, units/frames, targets and bindings | B01/B02 |
| `neuradix project compile` | Resolve immutable lock and target-specific plans/artifacts | B02 |
| `neuradix build`, `neuradix run` | Use the compiled project for supported host/target execution | B02/B07 |
| `neuradix embedded build`, `flash`, `monitor`, `size`, `test` | Actual board workflow with explicit target identity | B04/B05/C04 |
| `neuradix sim run`, `step`, `reset` | Native simulation API with pinned backend/assets | C01/C02 |
| `neuradix test replay` | Execute a selected pinned program on recorded input and compare results | A07/C06 |
| `neuradix test scenario` | Closed-loop scenario execution with declared tolerance/repeats | C05/C06 |
| `neuradix job submit`, `status`, `cancel` | Local/server worker runs with durable run identity | D01/D04 |
| `neuradix artifact inspect`, `verify` | Resolve provenance and verify retained artifacts | D02/E01 |

Exact arguments, schema fields and exit-code allocation must be reviewed before implementation. Do not add guessed exit codes to scripts. Long operations return stable operation/run identifiers and structured progress; cancellation and retry semantics are explicit. Keep display formatting separate from application logic.

Studio invokes these same services and validation rules. Context selects project/site/target explicitly; an offline local context must not require an enterprise login. Physical commands remain subject to freshness and authority even when a job or deployment operation can be retried.

# Compatibility and acceptance additions

- Snapshot the current CLI result envelopes/help and digest semantics before adding commands.
- Validate both CLI and Studio operations against common service fixtures.
- Preserve diagnostics identifying contract/layout/deployment revisions, profile capability errors and active target identities.
- Do not expose planned commands as available functionality; maintain command support status in release documentation.
- Validate clean-machine, offline, actual board, replay, disconnect and distributed cancellation paths through ACC-01/02/07/10/11/12/16.
