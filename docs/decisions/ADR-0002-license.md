# ADR-0002 — Workspace licence

- Status: Accepted
- Date: 2026-07-17

## Context

The functional specification states the intended licence for the open platform
core is Apache License 2.0, and the repository had no prior `LICENSE` file.

## Decision

Adopt **Apache-2.0** for the entire workspace. Every crate sets
`license.workspace = true`, and the repository root carries the full `LICENSE`
text with a `Copyright 2026 Busuttil Technologies Limited` notice.

## Consequences

- Permissive, patent-grant licensing consistent with the platform's open-core
  strategy (spec §37.4).
- New crates inherit the licence automatically via `[workspace.package]`.
