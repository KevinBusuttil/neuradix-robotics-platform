# RFC-0019 — Deployment Graph Validation

- Status: Partially implemented (foundation increment 8)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §6.3, §12.4 (EXEC-007), §16.1, §19.4, §28; complements RFC-0002, RFC-0005, RFC-0016, RFC-0018
- Crate: `neuradix-graph`; CLI: `neuradix graph validate`

## Problem

A deployment is a graph of components wired by contracts across nodes. The
platform's rule is **contracts before connectivity** (§3.1, §28.2): the topology
and its policies must be proven *before* anything is wired at runtime, not
discovered by a robot in the field. Several rules that are invisible to a single
crate become checkable only at the whole-deployment level:

- Python must stay off the deterministic control path (EXEC-007, §19.4) — the
  open question left by RFC-0018.
- An actuator may only be commanded through the Safety authority (§16.1).
- Every consumed contract must actually be produced by its declared producer,
  and every requirement must be satisfied by a connection (§6.3).
- The component graph must be acyclic and every component must be placed on a
  declared node.

## Scope

Implemented in this increment: a declarative deployment manifest
(`RobotDeployment`, `deploy.neuradix.io/v1alpha1`), an offline validator that
parses it into a typed model and reports every structural and policy problem in
one pass, a content-addressed **deployment identity**, and the
`neuradix graph validate <file>` command wired to exit code 10
(`DeploymentValidation`). Extended in a follow-up increment: **contract-reference
resolution** — with an authored contract registry (`--contracts <dir>`), every
wired reference is resolved to a real, validated schema and its content-addressed
`sha256:` identity is pinned in the report. Out of scope: actually launching a
deployment, cross-node transport selection, and resource/scheduling budgets.

## Proposed decision

### Manifest

A Kubernetes-style document: `apiVersion`/`kind`/`metadata`/`spec`, where `spec`
declares `nodes` (name + target), `components` and `connections`. A component has
a `node`, an `executionClass` (mirroring the runtime's spellings, RFC-0016), a
`runtime` (`rust`|`python`, default `rust`), a safety `role`
(`normal`|`safety`|`actuator`, default `normal`), and `provides`/`requires`
contract references. A connection is `from`/`to`/`contract`. A `contract` reference is a contract's
`namespace/name` identifier, optionally pinned as `namespace/name@major.minor.patch`.

### Contract-reference resolution (registry)

The graph checks contract *references* structurally on its own. Given a contract
registry — a directory of authored, validated contracts loaded via
`ContractRegistry::load_dir` — it additionally **resolves** every wired reference
to a real schema: a `namespace/name` with exactly one registered version resolves
unambiguously; `namespace/name@version` pins an exact one. Failures are issues,
not hard errors: `unknown-contract` (no such contract), `unknown-contract-version`
(contract present, version absent), `ambiguous-contract` (several versions, none
pinned — §28.4 immutability wants a pinned schema), and
`malformed-contract-reference`. Each successful resolution records the resolved
`namespace/name`, version and `sha256:` schema identity in the report, so a
deployment is pinned to the exact schemas it was validated against. This is the
bridge between RFC-0002 (schema identity) and this RFC.

### Validation is a report, not an error

Parsing failures (I/O, malformed YAML) are the only hard `GraphError`s.
*Everything else* — missing fields, unknown enums, policy violations — is a
`GraphIssue` (severity + stable kebab-case `code` + `path` + `message`) collected
into a `GraphReport`. This means a single pass surfaces **all** problems, and the
issue codes are a stable automation surface. `GraphReport::is_valid()` is false
iff any issue is error-severity.

### Checks

Structural: required fields, supported `apiVersion`/`kind`, duplicate
node/component names, known enum spellings. Graph-level: `unknown-node`
placement; `python-in-deterministic-path` and `python-feeds-deterministic-path`
(EXEC-007); `unknown-endpoint`, `provider-mismatch`, `consumer-mismatch` and
`unresolved-requirement` (contract wiring); `actuator-authority-bypass` and
`missing-safety` (§16.1); and `prohibited-cycle` via a deterministic
depth-first search with grey/black colouring that reconstructs the offending
cycle path.

### Deployment identity

WP-A08 replaces the structural-only hash with versioned declared and resolved
identities. Valid topology checks expose declared-v2; complete registry-aware
checks additionally expose resolved-v2, binding canonical configuration and computed
schema/scalar-layout identities. Invalid graphs expose neither. Raw model labels
are not validation evidence. See [A08 policy and migration](../implementation/WP-A08-Resolved-Deployment-Identity.md).

### CLI

`neuradix graph validate <file>` reports the identity, counts and issues in the
standard versioned envelope (RFC-0013). A valid deployment exits 0 (warnings are
surfaced but non-fatal); an invalid one exits **10** (`DeploymentValidation`); an
unreadable file exits 1.

## Public interfaces affected

`neuradix-graph`: `Deployment`/`Node`/`Component`/`Connection`,
`ExecutionClass`/`Runtime`/`Role`, `RawDeployment`, `from_yaml`/`load_file`/
`validate`, `validate_with_registry`, `ContractRegistry`/`ContractEntry`/
`Resolution`/`RegistryError`, `GraphReport`/`GraphIssue`/`ResolvedContract`/
`Severity`, `deployment_identity`, `GraphError`. CLI: the `graph validate`
subcommand and its `--contracts <dir>` flag. `neuradix-cli` depends on
`neuradix-graph`; the graph crate depends only on `neuradix-contracts` (a leaf),
so the layering stays acyclic.

## Alternatives considered

- **Fail fast on the first issue.** Rejected: an operator fixing a manifest wants
  the whole list, not a whack-a-mole of one error per run.
- **Model policy violations as `Result::Err`.** Rejected: it forces one problem
  per pass and conflates "cannot parse" with "parsed but invalid".
- **Reuse the runtime's `ExecutionClass` type directly.** Deferred: the graph
  crate is intentionally dependency-free of the runtime so the offline compiler
  stays a leaf; the spellings are asserted to match via RFC-0016 and shared
  fixtures rather than a code dependency.

## Safety and security implications

These are offline declarative checks. Safety role labels and adjacency do not
grant actuator capabilities, validate physical driver permissions or prove a runtime
path is enforced. Runtime authorization and physical acceptance remain separate.

## Compatibility implications

The manifest is versioned by `apiVersion`; new fields are additive and new checks
add issue codes without removing existing ones. `GraphIssue`/`GraphError` may gain
variants additively. The `sha256:` identity scheme matches contract schema
identity (RFC-0002).

## Testing strategy

`crates/graph/tests/validate.rs` derives every invalid variant from one valid
baseline, so each test proves a *single* policy flips validity, and asserts
identity stability under reordering. `crates/graph/tests/registry.rs` covers
resolution (pinned, unpinned-single, unknown, unknown-version, ambiguous,
malformed) and registry-aware validation. `crates/cli/tests/graph.rs` covers the
reference deployment (exit 0), full registry resolution (`--contracts`, four
schemas resolved), an unresolved reference (exit 10), an actuator-bypass manifest
(exit 10) and a missing file (exit 1). `examples/reference-auv/deployment.yaml`
wires the authored `contracts/standard/` contracts by their pinned references.

## Unresolved questions

- Broader codec/layout selection beyond A08 scalar-v2 resolution; schema change flips the deployment identity.
- Structural/semantic *compatibility* between a producer's and consumer's schema
  beyond reference equality (e.g. field superset/subset rules).
- Cross-node transport selection and resource/scheduling budget validation (§28).
- Warning-severity advisories (e.g. an unconnected `provides`, an unreachable
  component) beyond the current error set.
- Actually compiling a validated graph into a runnable deployment.

## A08 resolved identity API amendment

GraphReport now has immutable accessors: identity()/resolved_identity() return
optional versioned strings; issues()/resolved() return borrowed evidence. The
unchecked public deployment_identity helper is removed. CLI identity remains
explicitly declared-v2 and adds resolvedIdentity; invalid reports use null. Old
unversioned pins require revalidation, not reinterpretation. Configuration is a
bounded object with exact integers, strings, bool/null, arrays and string keys;
floats/tags/unknown manifest fields reject explicitly. All declared port references
resolve, and variable-length scalar layouts reject. Duplicate declarations fail.
The shared contracts::layout primitive preserves scalar v2 wire bytes and leaves
graph dependent only on contracts. See the A08 evidence document for exact limits,
canonical preimage and remaining delayed-feedback/runtime enforcement work.
