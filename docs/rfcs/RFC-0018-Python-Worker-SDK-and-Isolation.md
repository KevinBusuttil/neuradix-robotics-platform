# RFC-0018 — Python Worker SDK and Process Isolation

- Status: Partially implemented (foundation increment 7)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §19, §41.6; complements RFC-0017
- Crate: `neuradix-python`; Python library: `python/neuradix_worker.py`

## Problem

Python must be first-class for AI/analysis but isolated from safety-critical
paths (§3.7, §19.4). The defining requirement is crash safety (§41.6): a Python
component must be able to crash **without terminating control and safety
processes**. Python must also stay out of the deterministic control path
(EXEC-007).

## Scope

Implemented in this increment: a Rust-side supervisor that runs a Python
component as an **isolated OS process** over a line-delimited JSON protocol, with
health reporting, request timeouts, crash isolation and bounded restart; and a
native-feeling Python `run(handler)` library. Out of scope for this increment:
in-process PyO3/Maturin bindings and NumPy zero-copy views (§19.1–§19.2),
content-addressed/locked dependency environments, GPU/memory supervisor limits,
and wheel packaging.

## Proposed decision

### Isolation model

A Python component runs as a separate process (`python3 <script>`). It never
shares the runtime's address space, so a crash cannot corrupt the runtime — the
strongest form of the §19.4 isolation requirement.

### Protocol (newline-delimited JSON)

- startup handshake: worker → `{"kind":"ready","name","skipPolicy"}`;
- request: supervisor → `{"kind":"request","seq","payload"}`;
- response: worker → `{"kind":"response","seq","payload"}` or
  `{"kind":"error","seq","message"}`; `ping`/`shutdown` are also defined.
- stdout carries only protocol JSON; **stderr carries logs and tracebacks**, so a
  crash is visible and the supervisor observes it as a clean stdout EOF.

### Supervision

A background reader thread turns the blocking pipe into a channel, so
`PythonWorker::send` can **time out** (`WorkerError::Timeout`) and a worker crash
surfaces as a recoverable `WorkerError::WorkerExited` — never a panic or a hang.
`PythonWorker::health()` maps a running process to `Healthy` and an exited one to
`Unavailable`. `WorkerSupervisor` adds a **bounded restart budget** so a flapping
worker cannot restart forever (mirrors the FDIR restart-storm rule, RFC-0017).

### Composition with FDIR

A worker's `HealthState` feeds `neuradix_safety::FdirMonitor`, so a Python crash
drives the system to a safe mode — the example shows crash → `Unavailable` →
FDIR `nominal → safe`, with the runtime surviving and restarting the worker.

### Python-side ergonomics

`python/neuradix_worker.py` provides `run(handler, name, skip_policy)`; a
component author writes an ordinary `handler(payload, config)` function. The
`skip_policy` declares whether input samples may be skipped (§19.4). Structured
config is delivered via `NEURADIX_WORKER_CONFIG`.

## Public interfaces affected

`neuradix-python`: `WorkerConfig`, `PythonWorker`, `ReadyInfo`,
`WorkerSupervisor`, `WorkerError`. Python: `neuradix_worker.run`.

## Alternatives considered

- **In-process PyO3 embedding.** Rejected as the *first* step: embedding Python
  in the runtime process defeats crash isolation. PyO3/Maturin remains the plan
  for the zero-copy *binding* boundary (client types, NumPy views), which is
  additive to this process model.
- **Shared-memory / socket transport instead of stdio.** Deferred: stdio line-JSON
  is portable, simple and sufficient for control-plane messages; large-buffer
  paths will use the shared-memory data plane later.
- **Unbounded restart.** Rejected: a crash-looping worker must not restart
  forever; the budget latches failure (§16.8).

## Safety and security implications

Process isolation is the core safety property: Python cannot crash control or
safety. Python is deliberately kept out of the deterministic executor (its
supervision uses wall-clock time and real processes, so it is non-deterministic).
Request timeouts prevent a hung worker from stalling the caller. Restart budgets
prevent restart storms.

## Compatibility implications

The JSON protocol is versionable via the `kind` field and additive fields.
Adding PyO3 bindings is a new, separate surface, not a change to this one.
`WorkerError`/`WorkerConfig` may gain variants/fields additively.

## Testing strategy

`crates/python/tests/worker.rs` spawns a real `python3` and covers request
round-trip + config passthrough, **crash isolation and recovery**, supervisor
restart-budget exhaustion, and request timeout. The tests skip cleanly if no
interpreter is available. The `python-worker` example demonstrates crash → FDIR
safing → restart end to end.

## Unresolved questions

- PyO3/Maturin bindings and NumPy zero-copy views (§19.1–§19.2); wheel packaging.
- Content-addressed, locked Python dependency environments (§19.4).
- Supervisor-enforced CPU/memory/GPU limits (§19.4).
- Graph-compiler detection of Python in a declared deterministic control path.
