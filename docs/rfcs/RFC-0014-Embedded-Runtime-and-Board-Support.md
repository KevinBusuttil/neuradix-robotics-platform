# RFC-0014 — Embedded Runtime, Board Support and Code Generation

- Status: Draft (design only — NOT implemented in foundation increment 1)
- Authoritative spec: [Embedded Profile Implementation Plan v0.1](../Neuradix_Embedded_Profile_Implementation_Plan_v0.1.md), [Implementation Plan v0.3](../Neuradix_Implementation_Plan_v0.3.md) §4
- Crates (future): `neuradix-embedded-core`, `neuradix-embedded-codegen`, `neuradix-embedded-transport`

> This increment implements no embedded crate, MCU firmware or Arduino
> projection. This RFC records the intended design so the foundation does not
> foreclose it. Nothing here is a claim of implemented capability.

## Problem

Constrained MCUs must participate in the same contracts, health, safety and
simulation ecosystem without running the full Linux runtime, and without leaking a
particular embedded executor into the SDK.

## Scope (future)

`no_std` component API; static topology; bounded memory; health/identity; local
safe state; serial/CAN transport with framing/CRC/sequence; generated Rust
`no_std` and Arduino/C++ endpoints; host-simulation parity; and the
`neuradix embedded` CLI (`targets`/`new`/`check`/`generate`/`build`/`size`/
`flash`/`monitor`/`inspect`/`test`/`provision`/`update`).

## Proposed decision (intended)

- **Tiers**: Embedded Tiny (generated Arduino/AVR C/C++), Embedded MCU (native
  `no_std` Rust), Embedded Connected, Embedded High.
- **First targets, in order**: host simulation → ESP32-C3 → RP2040 →
  STM32F4/G4 → Arduino Uno R3 (generated C++) → nRF52 → ESP32-S3 / Uno R4.
- **`embedded-core`**: executor-neutral static component trait, bounded ports,
  health, command lease, watchdog, deployment identity and safe-state interface.
- **Executor adapters**, in order: static-loop host simulator → Embassy → RTIC.
- **`embedded-codegen`**: `no_std` Rust, Arduino C++ and embedded C projections
  plus topology and memory-report generation, with golden encode/decode vectors.
- **Reference node**: an AUV propulsion node that validates a lease, enforces
  rate/current/thermal limits, reports health and enters a safe state on link
  loss — runnable as host simulation, ESP32-C3 firmware and generated Arduino C++.
- Wireless links are never treated as a safety channel; safe state is local.

## Boundaries respected by increment 1

- `neuradix-contracts` and `neuradix-time` are already dependency-light and do not
  require the full runtime, so `embedded-core`/`embedded-codegen` can reuse the
  same contract model and clock-domain vocabulary (the plan's rule
  "`embedded-core → contracts/time, no full runtime dependency`").
- The Rust code generator is structured so additional target projections
  (`no_std` Rust, C++) are new emitters over the same validated `Contract`.

## Public interfaces affected (future)

New `embedded-*` crates and an `embedded` CLI subtree, reusing the same
application services and result schemas as the desktop CLI (Studio/CLI parity).

## Alternatives considered

- **A common embedded runtime shared with Linux.** Rejected: Arduino compatibility
  forces a *generated endpoint projection*, not a shared runtime implementation.
- **Leak Embassy/RTIC types into the SDK.** Rejected: `embedded-core` stays
  executor-neutral; adapters bind to a specific executor.

## Safety and security implications

Every actuator controller defines a local safe output applicable without the host
(NRX-EMB-004); the embedded gateway validates contract version, integrity,
sequence, freshness and authority before applying commands (NRX-EMB-005). Signed
wired flashing precedes any production OTA.

## Compatibility implications

Contract projections must round-trip via golden encode/decode vectors so host,
`no_std` Rust and Arduino C++ agree on the wire. Target support levels and
conformance tests gate what "supported" means per board.

## Testing strategy (future)

The embedded conformance suite (Implementation Plan §7): encode/decode vectors,
timestamp/sequence handling, queue overflow, watchdog reset, lease expiry,
safe-state transition, health/identity, transport corruption detection, resource
budgets and host-simulation equivalence.

## Unresolved questions

- First native MCU target and first embedded transport (ESP32-C3 vs RP2040;
  serial vs CAN) — to be chosen before implementation.
- Static memory/timing budget expression in contracts.
- Deployment-identity representation in firmware.
