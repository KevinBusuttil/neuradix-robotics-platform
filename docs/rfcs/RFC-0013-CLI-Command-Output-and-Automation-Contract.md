# RFC-0013 — CLI Command, Output and Automation Contract

- Status: Accepted (partially implemented — foundation increment 1)
- Authoritative spec: [CLI Command Specification v0.1](../Neuradix_CLI_Command_Specification_v0.1.md), [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §33
- Crate: `neuradix-cli` (binary `neuradix`)

## Problem

The CLI is an automation surface. Its command hierarchy, machine-readable output
and exit codes must be stable so scripts and CI can depend on them, and operation
logic must be separable from parsing and rendering.

## Scope

Implemented in this increment: `version`, `doctor`, and `contract`
`validate`/`inspect`/`hash`/`generate`; the `--output table|json|yaml` selector;
the versioned result envelope; and the full documented exit-code set (values not
yet produced are still represented). The remainder of the command tree in the CLI
spec (`run`, `graph`, `record`, `replay`, `explain`, `sim`, `embedded`, …) is
reserved for later increments.

## Proposed decision

### Architecture

`parse (clap) -> request model -> application service -> result envelope ->
render`. Application services return structured data or a typed error and never
render; rendering never contains operation logic.

### Result envelope (`cli.neuradix.io/v1alpha1`)

```json
{ "apiVersion": "...", "kind": "CommandResult", "command": "contract.validate",
  "status": "success|failure", "context": "local",
  "data": {}, "warnings": [], "errors": [] }
```

Volatile `startedAt`/`finishedAt` fields from the CLI spec are intentionally
omitted so output is deterministic and golden-testable without normalisation.

### Global flags

`--output/-o table|json|yaml` (default `table`), global across subcommands.
`table` is human-readable; `json`/`yaml` serialise the full envelope.

### Exit codes

The complete documented set is represented (0 success, 1 general, 2 invalid use,
3 contract validation, 4 compatibility, 5 connectivity, 6 authentication,
7 authorization, 8 safety rejection, 9 determinism/replay, 10 deployment,
11 partial, 12 timeout). This increment produces 0, 1, 2 (via `clap`) and 3.

### `contract generate --out-dir`

Because the global `--output` selects the machine-readable format, `contract
generate` writes generated code to `--out-dir` (not `--output` as sketched in
§33). This is the one intentional flag-name divergence.

## Public interfaces affected

The command tree, global flags, envelope schema and exit-code set. The envelope
is mirrored for consumers by `neuradix-testkit::cli_output::ParsedEnvelope`.

## Alternatives considered

- **Reuse `--output` for the generate directory.** Rejected: it collides with the
  global format selector.
- **Include timestamps in the envelope.** Rejected for now: they make golden tests
  flaky; they can be added out-of-band later.
- **Let `clap` own all error exit codes.** Partially adopted: `clap` handles
  invalid usage (2) and `--help`/`--version` (0); command-level failures map to
  our typed codes.

## Safety and security implications

Live-mutation and actuator commands are out of scope here; when added they must
use authenticated authority and must not bypass onboard Safety (CLI spec §Safety).
Stable exit codes prevent scripts from misinterpreting failures as success.

## Compatibility implications

`apiVersion` on the envelope allows evolution. New commands and data fields are
additive. Exit-code meanings are fixed. `--out-dir` is documented as stable.

## Testing strategy

`crates/cli/tests/cli.rs` runs the built binary and asserts: well-formed JSON
envelope for `version`; successful validate/hash of the reference contract (with
a pinned schema identity); exit code 3 for an invalid contract; and human/YAML
rendering. Assertions use `neuradix-testkit::cli_output`.

## Unresolved questions

- `jsonl` streaming output and `--offline`/`--context`/`--dry-run` semantics.
- Shell-completion generation.
- Authentication/authority model for live-mutation commands.
