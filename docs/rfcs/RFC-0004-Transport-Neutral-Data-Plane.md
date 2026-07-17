# RFC-0004 — Transport-Neutral Data Plane

- Status: Accepted (partially implemented — foundation increment 1)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §9, §11, §13
- Crate: `neuradix-transport-api`

## Problem

Components must be written against a transport-neutral interface so that
shared-memory, Zenoh, CAN and serial backends can be added later without changing
component code, and so that no backend type (Tokio, Zenoh, DDS, PyO3, a raw
channel) leaks into the public API. Queues must be bounded by default with
observable overflow behaviour.

## Scope

In scope: the neutral bounded-stream interface and one in-process backend with
four overflow policies and observable statistics. Out of scope for this
increment: State/Command/Task/Event/Query primitives, network/shared-memory
backends, and the message envelope (§11.1).

## Proposed decision

- Traits `StreamPublisher<T>` and `StreamSubscriber<T>` are the neutral surface.
  Component code depends on these, never on a concrete type.
- `StreamConfig { capacity: NonZeroUsize, overflow: OverflowPolicy }`, buildable
  from a contract's `Delivery` via `from_delivery`. Capacity is always bounded.
- `OverflowPolicy` lives in `neuradix-contracts` and is reused here so the
  authored policy and runtime behaviour cannot drift. When the queue is full:

  | Policy        | Behaviour                                            | Counter    |
  |---------------|------------------------------------------------------|------------|
  | `reject`      | refuse the incoming item                             | `rejected` |
  | `drop-oldest` | evict the oldest queued item, enqueue the new one    | `dropped`  |
  | `drop-newest` | drop the incoming item                               | `dropped`  |
  | `keep-latest` | retain only the single most recent item (depth ≤ 1)  | `dropped`  |

- `PublishOutcome` reports which of the above happened per publish;
  `StreamStats` exposes `capacity`, `len`, `published`, `delivered`, `dropped`,
  `rejected`, `closed`.
- The in-process backend (`in_process::<T>()` returning `InProcessPublisher` /
  `InProcessSubscriber`) is implemented with a private `VecDeque` behind a
  `Mutex`; the queue/lock types are not part of the public API. Lock poisoning is
  recovered rather than propagated as a panic. No `unsafe` code.
- `close()` prevents further publishes (`StreamError::Closed`) but still allows
  queued items to be drained.

## Public interfaces affected

`neuradix-transport-api`: `StreamPublisher`, `StreamSubscriber`, `StreamConfig`,
`PublishOutcome`, `StreamStats`, `StreamError`, `InProcessPublisher`,
`InProcessSubscriber`, `in_process`.

## Alternatives considered

- **Return concrete channel types (`std::sync::mpsc`, `crossbeam`).** Rejected:
  that leaks the backend and breaks neutrality.
- **`keep-latest` as a sliding window of N.** Rejected: that duplicates
  `drop-oldest`; latest-value semantics (depth ≤ 1) is a genuinely distinct,
  useful policy.
- **Define `OverflowPolicy` in this crate.** Rejected: it is a contract-level
  delivery property; defining it in `contracts` keeps authored and runtime
  semantics identical and respects the dependency direction.

## Safety and security implications

Bounded queues with explicit, observable overflow uphold "bounded by default"
(§3.2, EXEC-001/002/003). Poison-recovering locks avoid a panic path in a data
plane that may sit near control loops. Backend neutrality keeps foreign
middleware assumptions out of the platform's public API (§3.8).

## Compatibility implications

Component code that uses only the traits is insulated from future backends.
Adding a backend adds new constructor(s) returning types that implement the same
traits. `StreamStats`/`PublishOutcome` may gain fields/variants (additive).

## Testing strategy

In-crate tests in `crates/transport-api/src/stream.rs` cover ordered flow, each
overflow policy, the capacity invariant under load, close-then-drain, and
`from_delivery`. `neuradix-testkit` provides reusable `publish_all`/`drain_all`
helpers exercised in its own tests.

## Unresolved questions

- The zero-copy loaned-buffer/lease design for large payloads (§11.3).
- Blocking vs non-blocking subscriber APIs and readiness signalling.
- How per-connection policy (deadline, priority, encryption — §13.4) is expressed.
