# RFC-0005 — Safety Authority and Command Lineage

- Status: Draft (design only — NOT implemented in foundation increment 1)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §16, §25.3–§25.4
- Crate (future): `neuradix-safety`

> This increment implements no safety crate, no authority path and no actuator
> commands. This RFC records the intended design so that the interfaces built now
> (component lifecycle, contracts, time, data plane) do not foreclose it. Nothing
> here is a claim of implemented capability.

## Problem

No ordinary component may directly and unconditionally drive a safety-relevant
actuator (§16.1). Actuator requests must pass an authority and constraint path,
and every actuation must be explainable back to its inputs.

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
