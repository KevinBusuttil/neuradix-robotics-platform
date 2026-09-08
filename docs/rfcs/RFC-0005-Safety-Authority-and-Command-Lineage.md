# RFC-0005 — Safety Authority and Command Lineage

- Status: Partially implemented (foundation increment 4)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §16, §25.3–§25.4
- Crate: `neuradix-safety`

> Increments 4–5 implement the **authority + constraint gate** with auditable
> [`SafetyDecision`] evidence, plus a **recorded command-lineage** and the
> `neuradix explain command` view (§25.3–§25.4). Still **future**: FDIR state
> machines (§16.8), independent safety-island deployment (§16.7), command
> arbitration beyond priority (§16.2 voting/pre-emption), and correlating lineage
> across *multiple* recorded channels (currently one self-describing lineage
> record per command).

## Problem

No ordinary component may directly and unconditionally drive a safety-relevant
actuator (§16.1). Actuator requests must pass an authority and constraint path,
and every actuation must be explainable back to its inputs.

## Implemented (increment 4, updated by A04.2)

The `neuradix-safety` crate provides:

- **`AuthorityLease` / `LeaseTable`**: a maximum of 32 trusted holder/capability
  bindings, one validated current session per binding, with optional permitted
  `CommandEnvelope`. Runtime time authorizes the lease; source age, deadline,
  generation and sequence are checked independently. Trusted renewal preserves
  accepted state; replacement requires a greater generation. Revocation retains
  the replay watermark. Same-binding priority grants are replaced by explicit
  trusted provisioning; arbitration remains future work.
- **`Constraint`**: validated private range and units-per-second slew limits, each
  carrying a stable rule id. A rate limit is a no-op on the first command (no
  reference value), so hard limits always govern — a constraint is never
  silently undone.
- **`SafetyGate`**: authorizes, then applies constraints in order, producing a
  `SafetyDecision { outcome: Accepted | Modified | Rejected, applied,
  acted_rules, .. }`. Rejection applies a configured **fail-safe** value. The
  gate is a `neuradix_runtime::Processor`, so decisions are deterministic and
  replay identically under the executor (RFC-0016).
- **`CommandLineage`** (increment 5): a self-describing, JSON-serializable record
  linking the originating sensor input → requested command → authority/constraint
  outcome → applied value. Recorded on the `safety/command-lineage` channel and
  read back by `neuradix explain command <recording> --at <nanos>`, which prints
  the causal chain for the command nearest the requested time.

## A04.1 trusted evaluation and configuration

`SafetyGate::new` is fallible; safe outputs must be finite and satisfy every hard
range. `evaluate(Some(request), now)` takes runtime-owned evaluation time, and the
`Processor` implementation passes `TickContext.now`. `CommandRequest.meta.source_at` remains
source metadata. Idle `evaluate(None, now)` ticks enforce held-command expiry. Clock-domain changes, time regression and elapsed-time overflow
latch local fallback until gate reconstruction. Non-finite commands and invalid
constraint/final outputs produce typed rejections. Configuration cannot bypass
validated constructors through public fields/variants.

See [A04.1 policy, API migration and evidence](../implementation/WP-A04.1-Trusted-Evaluation.md).
[A04.2 policy and migration](../implementation/WP-A04.2-Command-Freshness.md)
define the shared no_std freshness/deadline/sequence/generation checks, trusted
durable restart allocation, explicit shared timeline, rejected-traffic watchdog
rules and required periodic scheduling. Sequence/generation identifiers are not
authentication. Lineage includes source metadata separately from runtime time;
non-finite requested values use explicit JSON markers. Idle decisions retain a
reason/time but produce no invented command lineage. Host/MCU slew alignment and
physical safe-response evidence remain open; this RFC and WP-A04 remain partial.
Gate A remains open.

## Scope (future)

Command authority via time-bounded leases; a constraint evaluator
(bounds/rate/slew/geofence/limits); the command path Planner → Authority Manager
→ Constraint Evaluator → Rate/Range/Slew Limiter → Actuator Capability → Hardware
Safety Layer (§16.2); FDIR state machine (§16.8); and causal command lineage for
`explain` (§25.3). Deployment as an independent safety island (§16.7) is a
topology enabled by clean interfaces.

## Proposed decision (intended)

- **Authority leases** (§16.3): holder identity, controlled capability, priority,
  issue/expiry, permitted command envelope, pre-emption and renewal policy;
  expiry triggers a defined safe action.
- **Constraint engine**: independently versioned rules; each modification or
  rejection identifies the responsible rule (NRX-SAF-003).
- **Command lineage**: every actuator command links originating samples,
  estimator outputs, the planner/controller decision, the authority decision, the
  constraint result and the final output — the data behind `neuradix explain`.
- Safety monitors remain enforceable when Ground/Fleet/Studio are unavailable
  (NRX-PLT-006) and when non-critical components fail (NRX-SAF-005).

## Boundaries respected by increment 1

- The `Component` lifecycle gates actuator-affecting behaviour on reaching
  `Active` (and, per §8.2, receiving authority) — the hook exists conceptually.
- Contracts already declare an `authority requirement` field position (§10.1) for
  later use, and carry content-addressed identity for evidence.
- `neuradix-time` gives every decision a domain-tagged timestamp for lineage.

## Public interfaces affected (future)

A `neuradix-safety` crate depending on `runtime`, `contracts`, `time` and
`frames`; the command primitive in the data plane; and an `explain` query in the
CLI/Studio.

## Alternatives considered

- **Enforce authority purely in application code.** Rejected: safety authority
  must be a platform boundary, not per-component discipline.
- **Single global safety monitor.** To be weighed against per-capability leases;
  the spec favours explicit leases and independent islands.

## Safety and security implications

This is the core safety boundary; correctness is paramount and will require
adversarial testing and fault injection. The platform provides mechanisms and
evidence only — never a certification claim (§16.6).

## Compatibility implications

Introducing the authority path must not require rewriting existing components; it
sits between planners and actuator capabilities. Lease/constraint schemas will be
independently versioned.

## Testing strategy (future)

Fault-injection scenarios (§34.3), lease-expiry safe-state tests, constraint
rejection tests, and replayable lineage/`explain` verification.

## Unresolved questions

- Lease arbitration policy details (exclusivity/priority/voting — NRX-SAF-002).
- Fail-silent vs fail-safe selection per hazard (NRX-SAF-006).
- Independent safety-island IPC and its trust boundary.
