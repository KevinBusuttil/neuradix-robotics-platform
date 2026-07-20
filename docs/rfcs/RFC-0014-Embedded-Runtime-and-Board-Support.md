# RFC-0014 — Embedded Runtime, Board Support and Code Generation

- Status: Partially implemented (increments 13–15 — `embedded-core` WP2, `embedded-transport` framing WP4, `embedded-codegen` WP1)
- Authoritative spec: [Embedded Profile Implementation Plan v0.1](../Neuradix_Embedded_Profile_Implementation_Plan_v0.1.md), [Implementation Plan v0.3](../Neuradix_Implementation_Plan_v0.3.md) §4 (Phase 3E)
- Crates: `neuradix-embedded-core`, `neuradix-embedded-transport`, `neuradix-embedded-codegen` (implemented)
- First target (chosen): **ESP32-C3** (RISC-V) with a **serial** link.

## Implemented in increment 15 (WP1 — embedded contract codegen)

`neuradix-embedded-codegen` adds target projections over the same validated
`Contract` the host generator uses: a **`no_std` Rust** payload struct and an
**Arduino/C++** header, each with fixed little-endian `encode`/`decode`, plus
deterministic **golden encode/decode vectors** that anchor cross-language
agreement. The wire is a fixed layout (each scalar field in declaration order,
little-endian; variable-length fields rejected), so a frame's size is known at
compile time. Conformance is *executed*, not asserted: the generated C++ is
compiled with `g++ -Werror` and run against the golden vectors, and the
generated `no_std` Rust is compiled (`include!`) and run against the same
vectors — so the host reference, MCU Rust and C++ all agree byte-for-byte.
`neuradix contract generate --language nostd-rust|cpp` exposes both. Embedded C
and topology/memory-report generation remain future.

## Implemented in increment 13 (WP2 — embedded-core)

`neuradix-embedded-core` is now real: a `#![no_std]`, allocation-free,
executor-neutral component core for the Embedded MCU tier. It provides node and
deployment **identity**, the same **health** vocabulary as the host runtime, a
time-bounded **authority lease**, a link-loss **watchdog**, and a local
**command gate** that enforces authority → link → validity → envelope (range +
slew) and applies a **local safe output** on lease expiry, link loss or a
non-finite command (§16.1, NRX-EMB-004). The reference **`PropulsionNode`** is
built from these and runs unchanged in host simulation (`examples/embedded-
propulsion`). To make this parity real, **`neuradix-time` was made
`no_std`-compatible** (a default-on `std` feature gates only the ambient
`SystemClock`), so host and firmware share the identical `Timestamp` /
`Duration` / `ClockDomain` types.

## Implemented in increment 14 (WP4 — transport framing)

`neuradix-embedded-transport` provides the on-wire framing for a byte link
(serial-first), `#![no_std]`, allocation-free and dependency-free (only `core`):
a frame is `sync(2) | seq:u16 | len:u16 | payload | crc32:u32`. [`encode`] frames
into a caller buffer; [`FrameDecoder`] is a byte-at-a-time, resync-capable state
machine over a fixed buffer that yields only CRC-verified frames (a corrupt or
oversized frame is dropped as [`FrameEvent::Corrupt`] without overrunning the
buffer); [`SequenceTracker`] classifies each frame as in-order, duplicate,
gapped or reordered with wrap-safe arithmetic. Integrity (CRC) and ordering
(sequence) live here; **freshness** stays with the embedded-core watchdog — a
corrupt/missing frame is simply a missing command, which drives the node's local
safe state. An integration test drives a `PropulsionNode` through the codec and
shows sustained corruption → link-loss safe state.

Still future: contract projections and golden vectors (WP1), the Embassy/RTIC
executor adapters (WP3), CAN transport, the `neuradix embedded` CLI and real
board builds/flashing (WP5). The board/transport target is now chosen (ESP32-C3,
serial); real cross-compilation awaits a toolchain and hardware.

> The remainder of this RFC records the intended design for the not-yet-built
> parts so the foundation does not foreclose them.

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
