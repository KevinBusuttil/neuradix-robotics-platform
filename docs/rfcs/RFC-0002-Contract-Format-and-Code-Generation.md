# RFC-0002 — Contract Format and Code Generation

- Status: Accepted (partially implemented — foundation increment 1)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §10, §15
- Crate: `neuradix-contracts`

## Problem

Every component, wire format, binding and tool depends on how a contract is
authored, validated, identified and projected into code. The contract model is
the hardest thing to change later, so its format, identity and first code
generator must be pinned down before an SDK is built on top of them.

## Scope

In scope for this increment: a human-authored YAML `StreamContract`; validation
with typed errors; a deterministic, content-addressed schema identity; and a
first Rust projection. Out of scope: State/Command/Task/Event/Query contracts,
Protobuf/JSON-Schema/Python/WIT projections, imports/versof-resolution across
contracts, and unit/frame *conversion* (only preservation is implemented).

## Proposed decision

### Authored format

```yaml
apiVersion: contracts.neuradix.io/v1alpha1
kind: StreamContract
metadata: { namespace, name, version }        # version is semver
spec:
  description: <non-normative text>
  payload:
    type: object
    fields: { <name>: { type: <primitive>, unit: <string?> }, ... }
  semantics: { frame, clockDomain, authoritativeTimestamp, maximumAge }
  delivery: { capacity: <u32 > 0>, overflow: <policy> }
```

- Supported primitive types: `float64`, `float32`, `int32`, `int64`, `uint32`,
  `uint64`, `bool`, `string`. Nested objects, arrays and maps are rejected with a
  clear error.
- Supported `clockDomain` values: `monotonic`, `utc`, `sensor`, `simulation`,
  `replay` (mirrors `neuradix-time`; see RFC-0003).
- Supported `overflow` values: `reject`, `drop-oldest`, `drop-newest`,
  `keep-latest` (see RFC-0004).
- `maximumAge` accepts duration literals (`ns`,`us`,`ms`,`s`,`m`,`h`; decimals
  allowed) normalised to whole nanoseconds.
- Units and frames are preserved verbatim as metadata; no conversion is performed.

### Divergences from the illustrative spec example (§10.3)

The spec shows `apiVersion: neuradix.io/v1alpha1` with `semantics.quantity/unit`
at the payload level and `overflow: drop_oldest`. This increment adopts the
concrete brief format instead: `apiVersion: contracts.neuradix.io/v1alpha1`,
per-field `unit`, and hyphenated overflow spellings. These are the only
intentional divergences; they are validated strictly and reported clearly.

### Schema identity

`schema_identity(&Contract) -> "sha256:<64 hex>"`. The identity is a SHA-256 over
a canonical JSON encoding of the *normative interface*:

- included: `apiVersion`, `kind`, `metadata` (namespace/name/version), payload
  fields (name, type, unit), semantics (frame, clockDomain, authoritative
  timestamp, `maximumAge` as a decimal-nanosecond string);
- excluded: `spec.description` (non-normative documentation) and `spec.delivery`
  (a QoS policy, not part of the wire schema).

Canonicalisation sorts all object keys, sorts payload fields by name, and emits
no insignificant whitespace, so the identity is independent of formatting and
field/key order. Field *declaration order* is preserved for code generation but
does not affect identity.

### Code generation

`generate_rust(&Contract)` emits a deterministic, `rustfmt`-clean Rust module:
a `pub struct` (deriving `Copy` only when all fields are `Copy`), field docs
carrying units, and `NAMESPACE`/`NAME`/`VERSION`/`SCHEMA_ID` associated
constants. Output is byte-for-byte reproducible and golden-file tested.

## Public interfaces affected

`neuradix-contracts`: `Contract`, `Metadata`, `Spec`, `Payload`, `Field`,
`PrimitiveType`, `ClockDomainRef`, `Semantics`, `Delivery`, `OverflowPolicy`,
`Duration`, `SchemaId`, `ContractError`, `ValidationIssue`, `validate::*`,
`schema_identity`, `canonical_bytes`, `generate_rust`, `SUPPORTED_API_VERSION`.

## Alternatives considered

- **Authored Protobuf as source of truth** (spec allows Protobuf for wire data).
  Rejected for v1: YAML is friendlier to author and diff; Protobuf becomes a
  *generated* projection later.
- **Include delivery/description in identity.** Rejected: doc edits and QoS
  changes should not invalidate a wire-compatible schema.
- **Sort fields for codegen too.** Rejected: declaration order is more natural
  for developers and future Protobuf field numbers; identity sorting already
  guarantees order-independent identity.

## Safety and security implications

Content-addressed identities let deployments pin exact schemas and detect drift
(supports the evidence/SBOM chain, §3.11). Validation is total and returns typed
errors; no panics on malformed input. Unbounded queues are rejected at validation
(`capacity > 0`), enforcing "bounded by default" (§3.2) at the contract layer.

## Compatibility implications

`apiVersion` is pinned to `contracts.neuradix.io/v1alpha1`; future versions add a
new value and an adapter. Adding a primitive type or clock domain is additive.
Changing canonicalisation changes identities and is a breaking change gated by an
`apiVersion` bump. Golden tests will catch accidental identity/codegen drift.

## Testing strategy

`crates/contracts/tests/validation.rs` (valid parse, all-issues collection,
missing sections, unbounded-queue rejection) and `crates/contracts/tests/golden.rs`
(golden Rust output; identity shape; identity invariance under reordering and
duration re-spelling; description-independence and field-type-dependence).

## Unresolved questions

- Exact rule set for unit/frame *conversion* generation (deferred).
- Whether `string` and other non-`Copy` fields need a distinct large-buffer path.
- Import/reference resolution between contracts.
