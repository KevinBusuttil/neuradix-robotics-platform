# RFC-0016 — Deterministic Execution

- Status: Accepted (partially implemented — foundation increment 3)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §12; complements RFC-0001 and RFC-0003
- Crates: `neuradix-runtime`, `neuradix-time`

## Problem

Deterministic replay is only credible if *execution* is deterministic, not just
data capture. Components must be driven by injected time in a defined order so
that the same inputs produce the same outputs — turning "the data replays
identically" into "the system replays identically".

## Scope

Implemented in this increment: a `ControllableClock` capability, an
input/event-driven deterministic executor (`Processor` + `run_lockstep`), and a
proof that a recorded run replays to identical outputs. Out of scope: a periodic
(rate-group) executor, real-time / fixed-priority scheduling, multi-component
graph scheduling, deadline/overrun monitoring and async/Tokio integration.

## Proposed decision

- **`ControllableClock`** (`neuradix-time`): a `Clock` whose time can be `set`
  or `advance`d. Implemented by `ManualClock`; deliberately *not* implemented by
  `SystemClock`, so a deterministic driver cannot be handed a non-reproducible
  clock by mistake.
- **`Processor`** (`neuradix-runtime`): a unit of component logic —
  `process(&mut self, ctx: &TickContext, input) -> Result<Vec<Output>, _>`.
  `TickContext` carries the executor-positioned `now` and a `sequence` index.
  Implementations must be deterministic for a given initial state.
- **`run_lockstep(clock, processor, inputs)`**: for each `(timestamp, input)` in
  order, positions the clock at `timestamp`, invokes the processor and collects
  outputs. Output is a pure function of `(initial state, inputs)`. Input
  timestamps must match the clock's domain (a typed error otherwise).

## Public interfaces affected

`neuradix-time`: `ControllableClock`. `neuradix-runtime`: `Processor`,
`TickContext`, `run_lockstep`.

## Alternatives considered

- **Periodic executor first.** Deferred: an input-driven executor maps directly
  onto record→replay (a recording *is* a time-ordered input sequence), so it
  proves the determinism claim with less machinery. The periodic executor shares
  this model and comes next.
- **Give `set`/`advance` to every `Clock`.** Rejected: `SystemClock` must remain
  uncontrollable so it cannot leak into deterministic paths.
- **Bake scheduling into the `Component` trait.** Rejected: lifecycle
  (`Component`) and processing (`Processor`) are separate concerns; a component
  may implement both.

## Safety and security implications

Deterministic, injected-time execution is the foundation for reproducible
incident analysis and for the future safety/authority path (RFC-0005), where a
command's lineage must be replayable. Barring ambient clocks from controllable
execution prevents silent nondeterminism entering control logic.

## Compatibility implications

`Processor`/`run_lockstep` are additive to the runtime. The periodic executor
will be an additional entry point, not a change to this one. `ControllableClock`
is additive to `neuradix-time`.

## Testing strategy

`crates/runtime/src/executor.rs` unit tests (determinism across two runs, clock
positioning, domain-mismatch error). `crates/runtime/tests/lockstep_replay.rs`
records a live run's inputs, replays them through a fresh processor under a
replay clock, and asserts identical outputs. The `minimal-depth-stream` example
demonstrates the same for a depth controller over the real contract type.

## Unresolved questions

- Periodic / rate-group scheduling and how "latest input" sampling interacts with
  determinism.
- Multi-component graph execution order and cross-component queues.
- Deadline / WCET monitoring and overload policy (EXEC-005/006).
- Async I/O boundary (Tokio) kept out of the deterministic executor (EXEC-007).
