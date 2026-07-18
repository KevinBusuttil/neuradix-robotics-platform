# RFC-0001 — Component and Lifecycle Model

- Status: Accepted (partially implemented — foundation increment 1)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §8, §12
- Crate: `neuradix-runtime`

## Problem

Supervision, safety, scheduling and health all hang off the contract between a
component and the runtime. That contract — identity, lifecycle states, legal
transitions and their audit trail — must be explicit and observable before any
executor or supervisor is built.

## Scope

In scope: stable component identity, the lifecycle state machine with a validated
transition table and audit records, an execution-class classification, a
structured health model, a minimal component manifest and the `Component` trait.
Out of scope for this increment: executors/schedulers, a distributed supervisor,
crash-loop quarantine, restart budgets and authority (COMP-003/004 and §8.2
authority are designed for but not implemented here).

## Proposed decision

- `ComponentId`: a non-empty, trimmed logical identity independent of process ID
  (COMP-007).
- `LifecycleState` = { `Declared`, `Configured`, `Inactive`, `Active`,
  `Degraded`, `Failed`, `Stopping`, `Stopped` }. Legal transitions:

  ```text
  Declared   -> Configured | Failed
  Configured -> Inactive   | Failed
  Inactive   -> Active      | Stopping | Failed
  Active     -> Degraded    | Stopping | Failed
  Degraded   -> Active      | Stopping | Failed
  Failed     -> Stopping    | Stopped
  Stopping   -> Stopped
  Stopped    -> (terminal)
  ```

- `Lifecycle` validates each transition and records a `TransitionRecord`
  carrying `from`, `to`, `reason`, `initiator` and a domain-tagged `Timestamp`
  (COMP-002). Illegal transitions return `LifecycleError::IllegalTransition` and
  do not mutate state. `current()` exposes state (COMP-001).
- `HealthState` = { `Healthy`, `Degraded`, `Unhealthy`, `Unavailable`, `Unknown` }
  (§25.5), with an optional reason (`HealthReport`).
- `ExecutionClass` = { `HardRealTime`, `Deterministic`, `Interactive`,
  `BestEffort`, `BatchAi` } (§12.1) — classification only in this increment.
- `ComponentManifest` carries id, name, version, execution class and the
  content-addressed schema identities it provides/requires (`neuradix_contracts::SchemaId`).
- `Component` trait: `id()`, default no-op `on_configure`/`on_activate`/
  `on_deactivate`/`on_stop` hooks and `health()`.

## Public interfaces affected

`neuradix-runtime`: `ComponentId`, `LifecycleState`, `Lifecycle`,
`TransitionRecord`, `HealthState`, `HealthReport`, `ExecutionClass`,
`ComponentManifest`, `Component`, `ComponentError`, `LifecycleError`.

## Alternatives considered

- **Ship the `#[component]` proc-macro now.** Rejected: stabilise the trait first;
  a premature macro ossifies a wrong API behind generated code.
- **Encode transitions as data (a table struct).** Rejected for now: an exhaustive
  `match` is clearer, is exhaustiveness-checked by the compiler and documents the
  table in one place.
- **Bundle authority into the lifecycle.** Deferred to RFC-0005; the model only
  records that actuator-affecting behaviour must not begin before `Active`.

## Safety and security implications

Explicit, validated, audited transitions make component state observable and
non-bypassable — the substrate for FDIR and safe-state handling. Recording
reason/initiator/timestamp on every transition supports the evidence chain
(§3.11). The lifecycle deliberately gates actuator behaviour on reaching `Active`
(and, later, receiving authority per §8.2).

## Compatibility implications

Optional states (`Calibrating`, `Standby`, `Updating`) from §8.2 can be added
additively. Adding execution classes or health states is additive. The transition
table is part of the runtime contract; changes are reviewed as breaking.

## Testing strategy

In-crate tests in `crates/runtime/src/lifecycle.rs`: a full legal bring-up/shut-
down sequence, rejection of illegal transitions without state mutation, terminal-
state enforcement, degraded↔active recovery and audit-field capture.
`neuradix-testkit::lifecycle` provides reusable transition assertions.

## Unresolved questions

- Restart policy/budgets and crash-loop quarantine (COMP-003/004).
- Readiness vs liveness separation (COMP-005).
- How the supervisor drives hooks and enforces production activation failure when
  contracts are unresolved (COMP-006).
