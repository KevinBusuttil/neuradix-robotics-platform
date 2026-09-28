# WP-A08 — Trusted actuator-driver permission binding

Status: host implementation integrated; [PR #20](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/20) merged as `7b2be979f94c9fdf7c9192c4a85f11cef1993340` on 28 September 2026; [post-merge CI 36361418592](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36361418592) passed. WP-A08 remains partial; Gate A remains open.

## Verified baseline and scope

Main `d4911fa38c9c7efa2cf006c4ae1a4c4694f92cb4` passed
[CI 36310554902](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36310554902).
PR #18 was already merged as `b35de66a16df27af4187c0479ed6c8965d3c0239`, with
[post-merge CI 35144276426](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/35144276426)
passing. PR #19 adds Manufacturing documentation only. No integration was repeated;
no open PR or applicable AGENTS.md was present. The isolated branch preserves other
checkouts and branches. A01's source/compiler baseline and A02's scalar identity
primitives are integrated prerequisites; their broader evidence inventory and
transport/compatibility acceptance remain incomplete.

Previously graph validation checked Safety/Actuator role adjacency, while the host
SafetyGate and embedded PropulsionNode returned decisions/scalars for callers to
apply. Neither owned a permission-guarded driver. Public SafetyDecision values are
constructible diagnostic data, not credentials or proof that physical output changed.

The new additive host module `neuradix_safety::actuator` owns one scalar actuator
endpoint and evaluates requests through the existing SafetyGate before invoking
its driver. This increment uses the existing thrust command model and a host test
driver. It introduces no firmware, hardware driver, graph executor, authentication,
transport or new dependency. Existing gate and no_std APIs remain compatible.

## Trusted ownership and admission

Trusted composition constructs an ActuatorBinding (holder/component, capability,
endpoint, execution mode), ActuatorConfig (hard range, slew rate, safe output),
and an ActuatorAdapter owning the driver. Configuration/grant state is private.
There is no process-wide physical endpoint registry; trusted composition must
ensure exclusive endpoint ownership across adapters/processes. Names are nonempty ASCII `[A-Za-z0-9_./:-]` of at most 128 bytes each; no trimming,
case folding or normalization. Numeric configuration is finite, bounds ordered,
rate nonnegative and safe value inside hard bounds. Initialization checks the
trusted driver implementation's fixed endpoint/mode descriptor before admitting it.
A descriptor is not hardware identity attestation.

Only trusted setup receives the adapter and creates/installs DriverPermission.
The grant binds an AuthorityLease to the exact holder, capability, endpoint and
mode. Graph roles, deployment hashes, Manufacturing requests and command fields
cannot install grants. The adapter exposes no driver/gate mutable accessor,
inner-driver extraction, Clone, or dispatch overload taking a SafetyDecision.
The owner must not retain an alias/raw driver handle that bypasses this boundary.
Rust API ownership cannot isolate malicious native code or prevent separately
opened OS/device handles: deployment access controls remain a separate obligation.

For each scheduled call, trusted composition creates a scoped, single-use
ActuatorPort with immutable mode and runtime evaluation time. Give components
only this port, never the adapter. Port input is an Option<CommandRequest>;
component payloads cannot choose evaluation time, select another mode, install
permissions or submit a precomputed output. The runner must supply true runtime
monotonic time for live operation; copying a source timestamp into port setup
would violate the integration contract. Call with None on periodic idle ticks.
Do not retain a port across scheduling intervals. Arbitrary component blocking,
driver methods and driver destructors require external supervision; this API
makes no wall-time guarantee for them.

Live uses Monotonic, Simulation uses Simulation, and Replay uses Replay clock
domains, respectively. Grants require the corresponding shared timeline; port
mode must equal endpoint mode. A replay/simulation port cannot dispatch requested
values to a live endpoint even if the request carries otherwise valid live metadata.
Mode is assigned by trusted composition, not deserialized from payloads. These
checks do not detect a dishonest producer relabelling historical bytes as new
live input: source authentication/provenance is outside this increment, and A04
freshness/generation/sequence checks retain their existing assumptions.

## State, evaluation and fallback policy

Construction performs no I/O. Trusted setup must keep equipment inhibited until
an initial safe write is acknowledged through grant or an idle/missing-permission
tick. Installing a grant always immediately selects/writes the configured safe
output and establishes the slew reference. Thus the first subsequent command is
rate-limited from safe, including zero elapsed time; this follows the existing
A04 rule for an initial idle/rejected evaluation. Safing is never delayed by slew.

There is one fixed binding for an adapter's lifetime. Replacement requires a
strictly greater lease Generation, including after revocation; duplicate/older
installation fails without changing state or writing. Rebinding endpoint, holder,
capability or mode requires a new adapter. Restart must durably reserve a newer
non-reused generation before ingress: use `ActuatorAdapter::new_reserved` with a
token from `neuradix_safety::reservation` and the Linux file store ([WP-A04.4](WP-A04.4-Generation-Reservation.md), proposed,
not merged). Renewal
extends a currently active lease without resetting source age, sequence, watchdog,
slew or evaluation-clock state. Replacement establishes safe output while retaining
the gate-wide clock and updates its slew reference to the actual safe selection.
Invalid trusted control times fail explicitly. Latched clock faults cannot be
cleared by grants, renewal or revocation.

Validation order:

1. Terminal shutdown; latched driver fault; port mode; permission presence/revocation;
   oversized source label rejection. All inhibited evaluations still observe the
   supplied runtime time, select safe output and clear held output without consuming
   a sequence or refreshing accepted-command liveness. Clock faults are reported
   separately. Invalid caller-owned requests are dropped, not retained in reports.
2. The unchanged A04 gate checks evaluation time, holder/capability, lease, command
   metadata, freshness/deadline/sequence, numerics, range/slew and final output.
3. Invoke the owned driver with that validated selection. Idle expiry and rejection
   select safe immediately. Every report distinguishes a gate decision from driver
   acknowledgment; an accepted command does not prove that a write took effect.

Revocation immediately attempts safe output and retains the generation watermark.
A missing/revoked/mismatched permission never authorizes a requested-value write;
the trusted setup still authorizes the fixed local safing path. An ordinary
rejection clears/overrides the held output, and recovery requires a new valid
command, rate-limited from the safe reference. Renewal alone cannot restore a
previous output after inhibition.

Driver errors use a bounded vocabulary: Unavailable, Rejected, UnknownOutcome.
Any failure latches until trusted reconstruction, even if a subsequent safe write
succeeds. Make exactly one immediate fallback attempt after a failed first write,
including failure of an initial safe write. Do not retry the requested output.
Later scheduled ticks attempt only safe output; at most two driver calls occur per
operation. The first fault remains auditable. A failed write can have unknown
physical effect, and failed safing cannot be described as reaching a physical safe
state. Device/controller protections remain necessary.

Shutdown is terminal: its first invocation attempts safe output, later calls perform
no driver I/O and cannot restore permission. The adapter makes no write call during
Drop. Explicit shutdown and external independent protections are the owner's
responsibility; arbitrary driver Drop code cannot be bounded here.

## Bounds and migration

WP-A04.4 (proposed) adds `DriverPermission::reserved`/`is_reserved`,
`ActuatorAdapter::new_reserved`/`requires_reserved_generations`,
`ActuatorBinding::reservation_key` and `PermissionError::{ReservationMismatch,
ReservationRequired}` (breaking only for exhaustive matches). A legacy adapter that
accepted a reserved grant refuses later unreserved grants. See [WP-A04.4](WP-A04.4-Generation-Reservation.md).

One binding/lease slot (including revoked watermark), three names of at most 128
bytes, exactly two scalar constraints, one fixed driver-error latch and no internal
queue/history. Each report copies the fixed binding (three bounded names), current trusted
generation and at most one admitted request (holder/capability at
most 128 bytes each), one SafetyDecision and at most two acted-rule IDs. Scalar
metadata/reasons and two driver results have fixed size. The gate allocates small
bounded vectors/strings per host evaluation; this is not an allocation-free host
API or a global allocator/RSS bound. Admitted label bytes are copied into fresh bounded ownership, so a short input
String with excessive spare capacity is not retained in the report. Incoming
allocation before admission, driver
implementation storage and caller-retained report history belong to their owners.
Rejected identities never create lease entries. Count tests exercise repeated
rejected grants and the single-slot limit. No embedded/no_std dependency changes.

Existing SafetyGate, CommandGate, graph identity versions, CLI and recording formats
are unchanged. SafetyDecision documentation now accurately calls it informational
and gate-selected, not immutable permission or physical-write evidence. New
integrations should use the owned adapter rather than sending decisions/raw scalar
values directly to a driver. Existing raw driver integrations do not automatically
become protected; they must migrate explicitly and remove bypass handles.

`cargo run --locked -p neuradix-safety --example guarded_actuator` demonstrates
missing permission, trusted installation, authorized output, replay-mode denial,
revocation and shutdown on an instrumented host test driver, with no device access.

## Verification evidence

Implementation `6ce893933dbbec28ce108ae3ebe72a1bf2d3c951` passed
[PR CI 36351856315](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36351856315).
Final head `6db84fba1921c8bc022aff9465b315061b68dfc4` passed
[PR CI 36352078681](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36352078681)
and [push CI 36352075505](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36352075505);
both host and AVR jobs passed in each run. PR #20 merged that head as
`7b2be979f94c9fdf7c9192c4a85f11cef1993340` with an identical tree;
[post-merge main CI 36361418592](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36361418592)
passed. Rust/Cargo 1.94.1, locked dependencies; all required host and separate AVR jobs
executed. Focused counts below are workspace subsets, not additional totals.

| Check | Result |
|---|---|
| Formatting and workspace/all-target Clippy with warnings denied | Passed |
| Guarded actuator suite | 10 passed: nine isolated scenarios plus subprocess entry; each scenario has a ten-second deadline |
| Private-grant, fixed-mode/time, single-use and forged-decision compile-fail examples | Five passed |
| Workspace tests/doctests | 378 passed |
| Command/safety/embedded/transport regression group | 103 passed, including the 15 new actuator tests/doctests and 88 preserved checks |
| Graph/CLI identity and delayed feedback | 39 graph plus six CLI tests/doctests and both examples passed |
| A07 replay | 20 tests/doctests and executable example passed |
| A05 workers and Python SDK | 75 Rust tests/doctests; six SDK tests passed |
| A06 recording | 44 Rust tests/doctests; independently pinned fixtures/export payload/schema/metadata/time/CRC verification passed |
| MCAP import peak RSS | Stream 4,224/4,376 KiB for 8/128 MiB payloads (64 MiB budget); materialization 135,132 KiB (192 MiB); bounded rejection 11,992 KiB (64 MiB) |
| MCAP export peak RSS | 3,028/2,988 KiB for 8/128 MiB payloads (64 MiB budget) |
| Guarded test-driver and existing control/embedded/Python examples | Passed |
| Independent no-default-features checks | time, command-core, embedded-transport, embedded-core all passed |
| Actual ATmega328P codec ABI conformance | Two passed in the separate AVR job; not execution of this host adapter on AVR |
| Workspace documentation generation | Passed |
| Local SDK; whitespace; changed Markdown links | Six SDK tests passed; whitespace passed; seven documents/325 links and anchors passed |
| Full Markdown inventory | 56 files/679 local links; 32 pre-existing archived missing-link failures, outside this scope |

Earlier formatting failures (36351310508/36351608429) and example timestamp type
inference failure (36351728981) were corrected; no checks were waived. The scoped
port API was strengthened during review to capture runtime time and consume the
port on submission. Report ownership copies admitted label bytes rather than
retaining arbitrary caller String capacity. The final implementation passed the
complete suite above; no physical timing or safety claim follows from it.

 Every actuator test
scenario runs in a separately timed ten-second subprocess; CI wraps the suite in
60 seconds and the example in 30 seconds. The broader workspace and existing
adversarial suites retain their external timeouts. No dependency/lockfile change.

Local Rust/Cargo/AVR tools are unavailable; direct toolchain retrieval timed out.
Git read access works but shell push lacks credentials, so GitHub plugin publishing
and pinned repository CI supply remote delivery/compiler checks. These tool limits
are not evidence that a code check passed. Physical rigs remain unavailable;
compiler/host tests do not constitute physical hardware validation.

## Remaining acceptance and Manufacturing

This demonstrates software permission separation for one selected host adapter.
WP-A08 remains partial: physical driver permissions require target/OS integration
and qualification, and graph execution/delay buffers/scheduling remain deferred.
The embedded PropulsionNode still returns a gate-selected scalar (now documented as
such); the corresponding allocation-free embedded owned-driver boundary is now
integrated through PR #22 (see [embedded permission evidence](WP-A08-Embedded-Actuator-Permission.md)).
`ExecutionMode` moved to command-core in that change and remains re-exported from
this module. Durable generation reservation for both boundaries is proposed in
[WP-A04.4](WP-A04.4-Generation-Reservation.md); a qualified board store remains open.

Manufacturing's integration note is a consumer requirement, not a live task API.
This does not implement its task authentication, durable outcomes, networking,
selected machine adapter or commissioning. No F03 work is pulled forward.
A01/A02 broader acceptance, A04/ACC-05 physical response, A05/ACC-09 broader resource
and platform containment, A06/ACC-08 broader interchange/scale, A07/ACC-07 closed-loop
evidence and Gate A's other requirements remain open.
