# WP-A08 — Explicit delayed-feedback topology validation

Status: implemented on `codex/a08-delayed-feedback-validation`; [PR #18](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/18) open and unmerged.
WP-A08 remains partial and Gate A open. This is offline validation, not execution.

## Baseline and defect

PR #17 is already merged as `11847e60010d67df636be067ece30ba19fff7da8`.
Its final head was `bbd36916f268dcb97a1ba2da55979f3622912853`; post-merge
[main CI 34390070855](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34390070855)
passed. GitHub main, PR and rulesets were checked before creating the isolated
branch. No reintegration or WP-A05 implementation changes were made.
WP-A01 supplies the integrated source/compiler baseline; its broader evidence
inventory remains partial. WP-A02 supplies validated schemas and shared scalar-v2
layout identity; its broader transport/compatibility acceptance remains partial.

Previously every directed cycle was rejected. This could not express a feedback
loop whose dependencies cross evaluation ticks. The validator now removes admitted
positive-delay edges for same-tick cycle analysis, while retaining **all** edges
for endpoint, contract, configuration and declarative policy checks.

## Normative manifest and validation policy

An omitted `delay` or `delay: instantaneous` is a same-tick dependency. Positive
feedback boundaries require exactly this mapping (key order is immaterial):

```yaml
delay:
  ticks: 1
  unit: evaluation-ticks
  initialization: require-seed
```

`ticks` is an unsigned integer in **1..=1024**, inclusive. Zero is rejected in a
mapping; use `instantaneous`. Only `evaluation-ticks` and `require-seed` are
supported. No wall-time unit conversion, fractional delays, implicit zeros or
sample-and-hold policy is inferred. Null, tags (including tagged member values),
floats, negative/overflowing numbers, unknown keys and missing initialization fail.
Duplicate YAML keys fail parsing. YAML-equivalent integer spellings canonicalize
to the same integer; strings containing digits are not integers.

A connection key remains `(from, to, contract)`. Repetition with the same delay
is `duplicate-connection`; a different delay is `conflicting-connection-delay`.
Neither establishes two independent channels. Invalid declarations produce
`invalid-connection-delay`, or a parse error where YAML cannot represent the value.
Invalid reports expose neither declared nor resolved identity. Conservative
instantaneous placeholders permit other checks to continue without granting validity.

The exact proved property is: **the subgraph of instantaneous edges is a DAG**.
Equivalently, every directed cycle includes at least one admitted positive-delay
edge. One delayed edge somewhere in the graph is insufficient: instantaneous
self-loops, disconnected loops and instantaneous subcycles still fail with
`prohibited-cycle`. This is a structural proof, not stability, causality of an
implemented scheduler, physical safety or closed-loop outcome evidence.

Traversal is iterative, with lexically sorted roots and neighbors. The first
cycle is deterministic under declaration reordering. Its diagnostic includes at
most 16 names, each truncated at a UTF-8 boundary to 32 source bytes, and a full
witness vertex count; the message is below 1 KiB. The transient witness has at
most 1,025 name references. Other existing validation messages retain their
existing policy and declaration order; this does not claim a new global report
byte limit for arbitrarily large programmatically supplied labels.

Existing bounds remain: 1 MiB manifest, 256 nodes, 1,024 components, 4,096
connections and 256 ports per component/list; configuration limits are unchanged.
The sum of positive delay ticks is at most **65,536 history slots**. One above
fails `delay-history-limit`. This is a count admission limit, not an allocated
runtime buffer or byte budget. Per-edge/count bounds make the u64 sum safe.
Cycle analysis storage is O(V+E), independently of the number of possible cycles;
sorting is O(V log V + E log E). Existing contract/wiring checks retain their costs.
No runtime path, allocation policy or embedded dependency changes.

## Required future initialization and scheduling

For edge delay D, consumer evaluation k uses producer output from tick k-D.
Before tick 0, trusted initialization must supply contract-valid history for
indices -D through -1, for every delayed edge. A delay annotation verifies only
that this obligation is declared. It does not supply or validate seed payloads.

Future execution must establish a common logical tick schedule, run the
instantaneous DAG in dependency order and preserve exactly the declared tick
association. The intended model has one frame per edge per tick; missing/extra
frames, skipped evaluations, restart and seed replacement must fail or follow an
explicitly validated future execution policy, never silently shorten the delay.
Cross-node graphs additionally require an established common schedule; offline
acceptance is not evidence of clock synchronization. No scheduler or delay buffer
is implemented here. A runtime unable to satisfy these obligations must refuse
execution. Logical tick progression is trusted runtime-owned state; command/source
timestamps never select evaluation time, authority or delay history. Existing
single-processor replay and A04 clock/lease guarantees are unchanged.

## Identity and API migration

`ConnectionDelay` has private invariant-bearing state. `Default` is instantaneous;
`new(ticks, unit, initialization)` validates positive delays. `ticks()` and
`is_instantaneous()` are read-only. `RawConnection` gains raw `delay`; typed
`Connection` gains validated `delay`. Rust struct literal callers must add these
fields. Manifest API remains `deploy.neuradix.io/v1alpha1` with an additive explicit
field: old manifests retain their meaning, old strict readers reject new delay
fields rather than silently ignoring them.

**No existing v2 preimage is changed.** All-instantaneous graphs (including explicit
instantaneous spelling) retain byte-for-byte v2 declared/resolved hashes and pins.
Graphs containing a positive delay use `neuradix.deployment.declared.v3:sha256:`
and, after complete registry resolution, `neuradix.deployment.resolved.v3:sha256:`.
Both the preimage `identityVersion` and digest prefix select v3. The existing
canonical JSON algorithm, sorted declarations, configuration, schema and layout
bindings remain normative. In v3, every connection adds `delay`: the string
`instantaneous`, or an object with integer `ticks`, string `unit` and string
`initialization` exactly as above. In v2 the field is absent. Changing a delay
changes both identities; changing authoring order/formatting does not. Removing
the last positive delay selects v2 only if the resulting graph validates.

CLI JSON reports `identityKind: declared-v2` or `declared-v3` for valid graphs,
null for invalid graphs, plus `topologyPolicy: instantaneous-dag-v1` and
`executionValidated: false`. Existing identity/resolvedIdentity fields and exit
codes remain. Consumers must accept v3 deliberately and replace affected pins;
do not relabel a v2 digest. Registry content is verified as before, while component
implementation, placement and delay obligations are caller-declared. Neither
identity nor a Safety role nor delay annotation grants actuator authority,
authenticates a component or attests an executable.

## Example and verification

`examples/delayed-feedback/deployment.yaml` uses standard scalar contracts for a
logical controller/plant topology. Run:

```sh
timeout 30s cargo run --locked -p neuradix-graph --example delayed_feedback
timeout 30s cargo run --locked -p neuradix-cli -- graph validate examples/delayed-feedback/deployment.yaml --contracts contracts/standard
```

The executable validates/resolves the delayed loop, then changes its feedback edge
to instantaneous and proves rejection. It does not execute a plant or controller.
Each new adversarial graph scenario has an independent ten-second subprocess
deadline; CI additionally times graph/CLI suites and examples. Cases cover delayed
and instantaneous loops, mixed/disconnected cycles, deterministic bounded witnesses,
1024-tick/65536-slot/4096-edge boundaries, invalid syntax, conflicts, seed obligations,
resolved v3 ordering/change behavior and historical v2 pins.

Implementation revision `86a7a792a771429ffa3380a60871f4d91414fcbd` passed
[PR CI 34403559830](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34403559830).
PR #18 records the final-head checks after the added delayed-edge policy assertions
and v3 golden pin. The run used Rust/Cargo 1.94.1 and locked dependencies; there
are no dependency or lockfile additions.

| Check | Result |
|---|---|
| `cargo fmt --all --check`; Clippy workspace/all-targets with warnings denied | Passed |
| Graph tests/doctests plus CLI graph suite, with external 60/30-second limits | 39 + 6 passed |
| Resolved identity and delayed-feedback executable examples | Passed |
| Workspace tests/doctests, external 180-second limit | 363 passed (focused counts are subsets, not additional totals) |
| Preserved A04 command/safety/embedded conformance | 88 passed |
| Preserved A05 bounded I/O/heartbeat/resource tests; SDK | 75 Rust tests/doctests; six Python tests passed |
| Preserved A07 runner and executable example | 20 tests/doctests and example passed |
| Preserved A06 import/export, independently pinned Python reader/fixtures | 44 Rust tests/doctests; independent payload/schema/metadata/time/CRC checks passed |
| MCAP memory regression budgets | Stream import 4,156/4,208 KiB for 8/128 MiB against 64 MiB; materialization 135,104 KiB against 192 MiB; rejection 11,920 KiB against 64 MiB; writer 3,012/2,860 KiB against 64 MiB |
| Migrated control/embedded/Python examples | Passed |
| Independent no-default-features checks: time, command-core, embedded-transport, embedded-core | All four passed |
| Separate actual ATmega328P ABI/compiler conformance | Two passed |
| `cargo doc --locked --workspace --no-deps` | Passed |
| Local SDK and independent fixture regeneration | Six SDK tests and exact fixture hashes passed; mcap 1.3.1, lz4 4.4.4, zstandard 0.23.0 |
| Changed Markdown links/anchors | Passed; eight documents scanned |
| Full Markdown inventory | 32 pre-existing archived missing-link failures, unchanged; archive repairs deferred |

Initial CI 34403212092 failed formatting and a new test fixture that retained an
unconnected requirement after removing its edge. Pinned formatter output and the
fixture were corrected; no gate was waived. Local Rust/Cargo and AVR executables
are unavailable in the editing environment; the above compiler results are from
GitHub CI. Physical rigs, response/timing and stack measurements are unavailable.
Host/compiler checks are not physical hardware validation.

## Remaining acceptance and next dependency

This increment demonstrates the legal delayed-loop versus instantaneous-loop
acceptance item. It does not implement deployment execution, delay scheduling or
buffers. WP-A08 remains partial for separation/enforcement of runtime capabilities
and physical driver permissions: a Safety label alone must never acquire access.
That bounded authority separation is the next A08 priority after review/integration.
Physical driver evidence remains necessary for any physical enforcement claim.
A04/ACC-05 physical evidence, A05/ACC-09 broader resources/platforms, A06/ACC-08
interchange/scale, A07/ACC-07 closed-loop evidence, A01's inventory and Gate A's
other criteria remain open. No package or gate is completed by this change.
