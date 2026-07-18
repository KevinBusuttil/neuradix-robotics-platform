# ADR-0001 — Rust toolchain pin and edition

- Status: Accepted
- Date: 2026-07-17

## Context

Reproducible builds and qualification-oriented evidence require a pinned,
documented toolchain. The workspace targets Rust edition 2024 (which requires
rustc ≥ 1.85).

## Decision

- Pin the toolchain in `rust-toolchain.toml` to channel **`1.94.1`** with the
  `rustfmt` and `clippy` components and the `minimal` profile. `1.94.1` is the
  exact stable toolchain validated in the development environment for this
  increment.
- Use **edition 2024** and Cargo **resolver 3** across the workspace, with a
  shared `rust-version = "1.94"` in `[workspace.package]`.

## Consequences

- CI and contributors build with the same compiler; `neuradix doctor` reports the
  pinned channel.
- Bumping the toolchain is a deliberate, reviewed change to `rust-toolchain.toml`
  and this ADR.
- Edition 2024 reserved keywords (e.g. `gen`) are avoided in identifiers.
