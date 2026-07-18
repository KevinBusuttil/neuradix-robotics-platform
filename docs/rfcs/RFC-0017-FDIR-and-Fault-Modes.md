# RFC-0017 — FDIR and Fault Modes

- Status: Accepted (partially implemented — foundation increment 6)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §16.5, §16.8; complements RFC-0005 and RFC-0016
- Crate: `neuradix-safety`

## Problem

A dependable robot must handle faults explicitly and testably: detect a fault,
confirm it (not react to transient glitches), escalate to a reduced or safe mode,
recover when it can, and avoid restart storms and repeated unsafe recovery
attempts (§16.8). This must be deterministic so a fault sequence replays.

## Scope

Implemented in this increment: a single-component fault-mode state machine
(`Nominal`/`Degraded`/`Safe`) driven by `HealthState`, with confirmation
debounce, a restart-storm budget, and an explicit operator return-to-service.
Out of scope: multi-component supervision and quarantine (COMP-004), redundancy
management / voting (§21.7), watchdog and deadline monitors as inputs, and the
independent safety-island partition (§16.7).

## Proposed decision

- **`FaultMode`** = `Nominal` | `Degraded` | `Safe`.
- **`FdirMonitor::observe(health, at)`** maps a `HealthState` to a severity
  (`Healthy` → ok; `Degraded` → soft; `Unhealthy`/`Unavailable`/`Unknown` → hard,
  treating absence of positive health conservatively) and applies the FDIR
  phases:
  - **detection + confirmation**: a fault must persist for `confirm_threshold`
    consecutive reports before it escalates (debounces glitches);
  - **accommodation**: a confirmed soft fault enters `Degraded`; a confirmed hard
    fault latches `Safe`;
  - **recovery**: a healthy streak of `recovery_threshold` reports recovers
    `Degraded → Nominal`, consuming one credit from a `max_recoveries` budget;
  - **restart-storm prevention**: once the budget is spent, the next recovery
    attempt latches `Safe` instead of flapping;
  - **return-to-service**: `Safe` never auto-recovers; `reset(at)` is an explicit,
    authorised operator action (§16.8).
- Each mode change yields an auditable `FdirTransition { from, to, at, reason }`.
- The monitor is a `neuradix_runtime::Processor`, so fault handling is
  deterministic and replays identically under the executor (RFC-0016).

## Public interfaces affected

`neuradix-safety`: `FaultMode`, `FdirPolicy`, `FdirMonitor`, `FdirTransition`.

## Alternatives considered

- **React immediately to any non-healthy report.** Rejected: transient glitches
  would cause needless safing; confirmation thresholds debounce them.
- **Auto-recover from `Safe`.** Rejected: returning to service after a hard fault
  is an operational decision, not an automatic one.
- **Unlimited recoveries.** Rejected: a flapping component would restart forever;
  the budget latches `Safe` (§16.8 "prevent restart storms").

## Safety and security implications

FDIR is a core safety function: it must be conservative (absence of health is a
fault), debounced (no chattering), and storm-resistant. Determinism means a field
fault sequence can be recorded and replayed to reproduce the exact safing
behaviour. The gate (RFC-0005) and FDIR compose: FDIR decides the mode; the
command gate enforces authority/limits per command.

## Compatibility implications

`FaultMode`/`FdirPolicy` are additive. Extending to multi-component supervision
adds new types (a supervisor over many monitors) rather than changing this one.
Additional fault inputs (watchdog, deadline miss) map to the same severity model.

## Testing strategy

`crates/safety/tests/fdir.rs`: debounce of a transient glitch, confirmed
soft→`Degraded`, confirmed hard→`Safe`, recovery after a healthy streak,
restart-budget exhaustion latching `Safe`, `Safe` requiring an operator reset,
and executor-driven determinism. The `minimal-depth-stream` example drives a
representative health sequence and prints the mode transitions.

## Unresolved questions

- Multi-component supervision, isolation and quarantine (COMP-004).
- Redundancy management and voting (§21.7); common-cause fault detection.
- Watchdog/deadline-miss and emergency-stop inputs feeding the same model.
- How FDIR mode interacts with command authority (e.g. `Safe` revoking leases).
