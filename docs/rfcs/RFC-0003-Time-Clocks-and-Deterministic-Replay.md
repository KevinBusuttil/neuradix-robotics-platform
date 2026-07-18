# RFC-0003 — Time, Clocks and Deterministic Replay

- Status: Accepted (partially implemented — foundation increment 1)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §14
- Crate: `neuradix-time`

## Problem

Deterministic replay — a headline platform differentiator — is only achievable
if time is explicit and injectable. Timestamps must carry their clock domain so
that mixing domains is impossible by construction, and deterministic logic must
never read ambient wall-clock time.

## Scope

In scope: clock domains, domain-tagged timestamps, a signed nanosecond duration,
an injectable `Clock` trait, a non-deterministic `SystemClock` and a
deterministic `ManualClock`. Out of scope for this increment: a recording-driven
replay clock, clock synchronisation/holdover reporting, and a thread-safe
simulation clock.

## Proposed decision

- `ClockDomain` = { `Monotonic`, `Utc`, `Sensor`, `Simulation`, `Replay` }. This
  is a subset of spec §14.1; `synchronized_monotonic` and GNSS time are reserved
  and currently unsupported. The spelling set is identical to the `clockDomain`
  vocabulary accepted by `neuradix-contracts`.
- `Timestamp { domain, nanos: i128 }`. Nanoseconds are counted from a
  domain-defined epoch. `duration_since` and `compare` require matching domains
  and otherwise return `TimeError::DomainMismatch`; equality across domains is
  simply `false`. There is intentionally no cross-domain `Ord`.
- `Duration` is a signed `i128` nanosecond value with an exact (no floating
  point) parser for `ns`/`us`/`ms`/`s`/`m`/`h` and decimal magnitudes; `100ms`
  and `0.1s` are equal.
- `Clock { domain(); now() }`. `SystemClock::monotonic()` / `::wall()` read
  ambient OS time and are documented as *not* for deterministic logic.
  `ManualClock` holds an interior-mutable (`Cell`) nanosecond value advanced by
  `advance`/`set`; it needs no sleeping and no ambient clock, making tests and
  simulation fully deterministic.

## Public interfaces affected

`neuradix-time`: `ClockDomain`, `Timestamp`, `Duration`, `Clock`, `SystemClock`,
`ManualClock`, `TimeError`.

## Alternatives considered

- **Reuse `std::time::{Instant, Duration}` directly.** Rejected: `std::Duration`
  is unsigned and panics on overflow, and `Instant` cannot carry a domain.
- **`ManualClock` with `&mut self` advance.** Rejected: interior mutability lets a
  simulation driver advance a clock that components hold by shared reference.
- **Depend on `neuradix-contracts` for the domain enum.** Rejected: `contracts`
  is the dependency root; the two crates keep independent but identical
  vocabularies, enforced by a `neuradix-testkit` test (see below).

## Safety and security implications

Making domain a compile-time-carried property of every timestamp removes a whole
class of silent time-base bugs (e.g. subtracting sim time from wall time).
`SystemClock`'s ambient reads are quarantined behind an explicit constructor and
documentation so deterministic/safety logic cannot use them by accident.

## Compatibility implications

Adding clock domains is additive but must stay in sync with the contracts
vocabulary; the sync is asserted by a test. `Timestamp`/`Duration` are stable
value types. A future thread-safe simulation clock will be an additional type,
not a change to `ManualClock`.

## Testing strategy

`crates/time/tests/time.rs`: duration parsing/normalisation, deterministic
`ManualClock` advance, cross-domain arithmetic errors, in-domain comparison, and
domain-checked `set`. A cross-crate test asserts `ClockDomain::ALL` equals the
contracts clock-domain vocabulary (planned in `neuradix-testkit`).

## Unresolved questions

- Representation of a recording-driven replay clock and lockstep stepping.
- Whether a thread-safe (`Sync`) simulation clock uses atomics or a mutex.
- How synchronisation quality/holdover (spec §14.3) is surfaced.
