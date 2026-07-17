# ADR-0003 — `neuradix-testkit` dependency set

- Status: Accepted
- Date: 2026-07-17

## Context

The implementation plan sketches `testkit → contracts, time and runtime`. The
task brief additionally requires `testkit` to provide reusable **bounded-stream**
test utilities, which need `neuradix-transport-api` types.

## Decision

`neuradix-testkit` depends on `neuradix-contracts`, `neuradix-time`,
`neuradix-runtime` **and** `neuradix-transport-api` (plus `serde`/`serde_json`
for CLI-output assertions). This is a superset of the sketch, justified by the
bounded-stream helper requirement.

## Consequences

- No dependency cycle is introduced: `transport-api → contracts` only, and nothing
  `testkit` depends on depends back on `testkit`.
- `crates/testkit/tests/architecture.rs` encodes the permitted internal
  dependencies for every crate (including this one) and fails CI if any crate
  gains a forbidden internal dependency.
- `cli` and the example depend on `testkit` only as **dev-dependencies**, so it
  never enters their normal dependency graph.
