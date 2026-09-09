# WP-A08 — Resolved contract and configuration deployment identity

Status: implemented on `codex/a08-resolved-deployment-identity`;
[PR #17](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/17) merged as `11847e60010d67df636be067ece30ba19fff7da8`; [post-merge CI 34390070855](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34390070855) passed. WP-A08 remains partial; Gate A remains open.

## Baseline and prerequisite review

PR #16 was reviewed at `410da4c99eda45947133fa0fdd1873e1759a9756`. Its current
PR CI 34380936422 and push CI 34380929961 passed. Main was unprotected, repository
rulesets were empty and no reviews/comments were present. Review covered factory
ownership, trusted evaluation time, exact comparison, admission/report bounds,
terminal failures and evidence. No concrete prerequisite defect required a change.
PR #16 was merged as `88c8eb1015a96d5314b1314b623758c7d43100f1` before this branch.
[Post-merge main CI 34382971652](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34382971652) passed.
Existing branches and user work were preserved in an isolated checkout. No AGENTS.md
instructions were found. PR #13/#14/#15 integration was not repeated; A05 code is unchanged.

WP-A01's integrated source/compiler baseline and A02's semantic schema/canonical
scalar v2 identities supply this increment's prerequisites. A01's broader evidence
inventory and A02's transport/compact-ID/recording acceptance remain partial.

## Defect and design

Previously `validate_with_registry` resolved wired references but hashed only the
manifest declarations. Changed schema contents behind the same reference did not
change deployment identity. Component configuration was not represented, and an
invalid graph still received an apparently usable identity.

Reports now distinguish `neuradix.deployment.declared.v2:sha256:<hex>` from
`neuradix.deployment.resolved.v2:sha256:<hex>`. Both are absent for invalid graphs.
Valid topology-only validation yields only declared identity; complete registry
validation additionally yields resolved identity. A valid empty graph with an
explicit empty registry can have a resolved identity: there are no contract claims.
This is not proof of an executing or useful deployment.

Normative canonical JSON contains identityVersion and deployment. Deployment
contains name, profile, nodes (name/target), components (name/node/executionClass/
runtime/role/provides/requires/configuration), and connections (from/to/contract).
Node/component lists sort by name; port references lexically; connections by
from/to/contract. Resolved identity additionally includes contracts sorted by exact
authored reference, with reference, resolved identifier/version, schemaId, codecId
and wireId. All JSON object keys sort lexically; arrays retain order except the
explicitly unordered declaration collections. JSON uses compact serde_json escaping
and UTF-8, then SHA-256. Identity version is inside the preimage and outside the hash.
API version/kind are validated to their sole supported values. No source path,
registry traversal order, unused registry entry, whitespace or description is hashed.
References with different spellings remain distinct even if they resolve equally.
Unicode is not normalized. Future canonicalization/codec changes require explicit
identity-version migration; old pins are never silently treated as v2 pins.

The registry resolves all provides/requires and wired references. Unused declared
ports cannot hide an unknown contract. Existing missing/version/ambiguity errors
remain, and unsupported scalar layouts produce `unsupported-contract-layout`.
Duplicate nodes/components, port references or identical connections fail; duplicate
registry identifier/version fails even when contents match. Unknown manifest fields
and duplicate YAML mapping keys fail parsing rather than being silently discarded.

## Shared layout and trust boundary

The existing canonical `neuradix.scalar-le.v2` calculation moves to
`neuradix_contracts::layout`. Embedded codegen re-exports the same layout types and
uses the same calculation, retaining descriptor bytes, field order, schema IDs,
wire IDs and generated wire format. Width accumulation is checked. This keeps
contracts as the dependency root and graph dependent only on contracts; no new
workspace edge, dependency or lockfile change is needed.

This increment supports exactly that codec: fixed bool, 32/64-bit integer and
binary32/binary64 fields. Strings have a semantic schema but no supported scalar
layout, so registry-aware validation rejects them explicitly. Another codec cannot
be selected by a manifest label. Changing a supported field width changes the
computed layout and deployment identity. General layout selection and target ABI
admission are not implemented: a scalar binary64 layout is not evidence that an
Uno can represent it. Existing target-specific generator rejection remains intact.

Registry entries used for validation are computed from parsed/validated contracts;
callers cannot insert arbitrary digest entries into the registry. GraphReport fields
are private and exposed by immutable accessors; clearing an issue or editing a
binding cannot manufacture a valid report. Public typed/raw models remain authoring
inputs and are revalidated. Output strings can of course be copied/altered by callers;
they are not security tokens. Schema/layout content is computed; node/runtime/role/
configuration labels remain declarations. There is no binary attestation, transport
authentication, execution proof or actuator permission from these identities.

## Configuration and bounds

`ComponentConfiguration` owns immutable canonical JSON with a checked constructor.
Omitted configuration is `{}`; explicit null at the root is invalid. Nested values
support null, booleans, exact signed i64/unsigned u64 integers, UTF-8 strings, ordered
arrays and string-key objects. Floats (including finite values, signed floating zero,
NaN/infinity), out-of-range integers, tags and non-string keys are rejected. Decimal
control values can be authored as strings with a component-defined exact meaning;
this layer does not coerce or validate component-specific semantics. Integer YAML
spellings such as 0x10 and 16 canonicalize equally; numeric strings remain distinct.

Inclusive fixed policy: 65,536 compact JSON bytes per component, 1,024 values
including the root (keys are byte-accounted, not value-counted), depth 16 with root
at zero; 1 MiB total admitted canonical configuration. Counting/escaped-byte checks
precede canonical copying. Raw programmatic inputs are caller-owned. Validation may
materialize per-component configurations before rejecting the total; with at most
1,024 components this temporary canonical data is at most 64 MiB, not a 1 MiB peak
RSS promise. Invalid configuration never receives an identity.

Manifest text is capped at 1 MiB before YAML parsing; file reads take at most that
plus one detection byte. Graph admission caps 256 nodes, 1,024 components, 4,096
connections and 256 entries per provides/requires list before typed-model copying.
These bound graph counts, canonical configuration and traversal depth, not arbitrary YAML parser,
registry loading, caller data or allocator RSS. Programmatically supplied raw model
labels/reference strings have no new byte cap; trusted callers must bound those
inputs themselves. The source byte cap applies to the file/string parsing APIs. Configuration nesting is checked
before recursive conversion. Run parsing/compilation outside conventional local
control with external supervision; no wall-time or whole-process sandbox guarantee.

## API and CLI migration

- GraphReport fields become `identity() -> Option<&str>`, `resolved_identity()`,
  `issues()` and `resolved()` immutable accessors. Public unchecked
  `deployment_identity(&Deployment)` is removed; validate raw input instead.
- Component gains validated configuration; RawComponent gains a YAML configuration
  value. GraphError gains Limit. Unknown manifest fields now reject explicitly.
- CLI `identity` remains the declared field but is versioned or null; `identityKind`
  is `declared-v2`. New `resolvedIdentity` is versioned or null; resolved bindings
  add codecId/wireId. Exit 0 still allows valid topology-only checks; consumers
  requiring resolution must require non-null resolvedIdentity. Invalid graphs exit
  10 with both identities null. Parse/size failures exit 10; I/O failures exit 1.
- `WireLayout::for_contract` now returns contracts::layout::LayoutError. Generator
  entry points retain CodegenError through conversion, including LayoutOverflow.
  Descriptor and wire bytes are unchanged; existing codec/AVR goldens remain gates.
- Existing config-free manifests remain readable and receive new v2 pins. Old
  unversioned hashes are not reinterpreted; revalidate with the intended registry
  and approve replacement pins. Recording formats/replay CLI semantics are unchanged.

`cargo run --locked -p neuradix-graph --example resolved_identity` demonstrates
separate declared/resolved states and configuration drift on the reference graph.
It performs no deployment execution or authorization.

## Verification

Verified implementation `4b6fad3b616c1f2455a79e793053f3f40decc721`:
[PR CI 34384823142](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34384823142)
and [push CI 34384817885](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34384817885)
passed. Documentation-only follow-up revisions are checked before PR readiness;
PR #17 links their final-head results.

- Pinned Rust/Cargo 1.94.1, locked dependencies: formatting, all-target Clippy with
  warnings denied, workspace documentation and architecture checks passed.
- Graph: 31 tests/doctests (six supervised scenarios plus child helper); CLI graph:
  five tests; resolved_identity example passed. Empty resolved-v2 hash is pinned
  against independently computed Python 3.11 json/hashlib bytes.
- Workspace: 354 tests/doctests passed. Counts exclude repeated child result lines;
  the two generic-host ignored AVR tests passed explicitly in the separate AVR job.
- Preserved A04: 88, A05: 75, recording: 44, runtime/replay: 20 tests/doctests;
  Python SDK: six. Integrated examples, four independent no_std configurations,
  scalar wire/ABI goldens and actual AVR compiler conformance passed.
- Independent MCAP producer fixtures and export reader/checksum agreement passed.
  Import RSS at 8/128 MiB streaming payload: 4,156/4,140 KiB (64 MiB budget);
  128 MiB materialization: 134,940 KiB (192 MiB budget); bounded rejection:
  11,956 KiB (64 MiB budget). Writer RSS at 8/128 MiB output: 2,892/2,864 KiB
  (64 MiB budget). These preserve A06 qualification, not graph-parser RSS claims.
- Local independent fixture regeneration/check and six SDK tests passed.
  Changed Markdown: eight files, 333 local links/anchors, zero errors. Full scan:
  52 files/661 links, 32 pre-existing missing archived spec/image targets (failed,
  deferred). Whitespace check passed.

Earlier iterations failed formatting, a missed CLI LayoutError conversion and two
raw-model Option assignments in a new test. These were corrected. The initial AVR
package refresh failed on unrelated Chrome repository hash mismatches before any
AVR compile; signed Ubuntu-only refresh resolved that infrastructure blocker. No
required compiler gate is waived, and full successful runs supersede those attempts.

Focused tests use external 10-second
child deadlines; CI wraps graph tests (60s), CLI graph tests/example (30s each),
plus existing externally timed workspace and A04/A05/A06/A07 suites. Tests cover
schema/layout/configuration changes, canonical ordering, exact byte/value/depth
bounds, invalid numeric/tagged values, source cap, duplicates and unresolved states.
Independent MCAP, SDK, no_std, AVR and architecture checks are preserved. The AVR
job refreshes signed Ubuntu package sources only: unrelated preinstalled Chrome
repository index hash mismatches blocked installation. Package signature/hash
verification and required AVR compilation remain enabled.

Local Rust/Cargo/rustup and AVR tools are unavailable; GitHub CI provides pinned
Rust/Cargo 1.94.1 with locked dependencies. Physical rigs are unavailable and no
hardware response, stack or timing validation is claimed. Pre-existing archived
link failures are reported separately and not repaired in this increment.

## Remaining acceptance

At this increment, legal delayed feedback and instantaneous-loop distinctions were still missing; the [delayed-feedback follow-up](WP-A08-Delayed-Feedback-Validation.md) implements their offline validation. WP-A08 remains partial for
runtime role/capability/physical-driver enforcement remain open. Existing graph
role checks are declarative only. A07/ACC-07 remain partial for broader execution
and closed-loop evidence; A04/ACC-05, A05/ACC-09, A06/ACC-08 and Gate A retain their
missing acceptance. This content identity increment does not close those gaps.
