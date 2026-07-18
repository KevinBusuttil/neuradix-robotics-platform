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
(`DeploymentValidation`). Out of scope for this increment: actually launching a
deployment, cross-node transport selection, resource/scheduling budgets, and
verifying contract *schema identities* against a registry (the graph checks
contract *references* by name; RFC-0002 owns schema identity).

## Proposed decision

### Manifest

A Kubernetes-style document: `apiVersion`/`kind`/`metadata`/`spec`, where `spec`
declares `nodes` (name + target), `components` and `connections`. A component has
a `node`, an `executionClass` (mirroring the runtime's spellings, RFC-0016), a
`runtime` (`rust`|`python`, default `rust`), a safety `role`
(`normal`|`safety`|`actuator`, default `normal`), and `provides`/`requires`
contract references. A connection is `from`/`to`/`contract`.

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

`deployment_identity` canonicalises the normative content (name, profile, and
the nodes/components/connections with provides/requires and collections sorted)
to key-sorted JSON and returns `sha256:<hex>`. It is authoring-order independent,
so a validated deployment can be pinned for production immutability (§28.4). The
identity is always reported, even for an invalid deployment, so a diff tool can
compare two rejected drafts.

### CLI

`neuradix graph validate <file>` reports the identity, counts and issues in the
standard versioned envelope (RFC-0013). A valid deployment exits 0 (warnings are
surfaced but non-fatal); an invalid one exits **10** (`DeploymentValidation`); an
unreadable file exits 1.

## Public interfaces affected

`neuradix-graph`: `Deployment`/`Node`/`Component`/`Connection`,
`ExecutionClass`/`Runtime`/`Role`, `RawDeployment`, `from_yaml`/`load_file`/
`validate`, `GraphReport`/`GraphIssue`/`Severity`, `deployment_identity`,
`GraphError`. CLI: the `graph validate` subcommand. `neuradix-cli` gains a
`neuradix-graph` dependency; the graph crate depends on no internal crate.

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

This is a safety *gate*, not merely a linter: `actuator-authority-bypass` and
`missing-safety` refuse to bless a deployment where an actuator could be driven
around the Safety authority (§16.1), and the Python-path checks enforce
EXEC-007. Because validation is pure and offline (no transport, no process spawn,
no wall clock), it is deterministic and safe to run in CI and at deploy time.

## Compatibility implications

The manifest is versioned by `apiVersion`; new fields are additive and new checks
add issue codes without removing existing ones. `GraphIssue`/`GraphError` may gain
variants additively. The `sha256:` identity scheme matches contract schema
identity (RFC-0002).

## Testing strategy

`crates/graph/tests/validate.rs` derives every invalid variant from one valid
baseline, so each test proves a *single* policy flips validity, and asserts
identity stability under reordering. `crates/cli/tests/graph.rs` covers the
reference deployment (exit 0), an actuator-bypass manifest (exit 10) and a missing
file (exit 1). `examples/reference-auv/deployment.yaml` is the worked reference
manifest.

## Unresolved questions

- Verifying contract *references* resolve to real, compatible schema identities
  in a registry (bridges RFC-0002 and this RFC).
- Cross-node transport selection and resource/scheduling budget validation (§28).
- Warning-severity advisories (e.g. an unconnected `provides`, an unreachable
  component) beyond the current error set.
- Actually compiling a validated graph into a runnable deployment.
