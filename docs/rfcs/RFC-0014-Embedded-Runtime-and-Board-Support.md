# RFC-0014 — Embedded Runtime, Board Support and Code Generation

- Status: Partially implemented on main; board/runtime expansion remains planned. Updated after PR #7 on 8 September 2026.
- Governing documents: [Specification v0.6](../Neuradix_Robotics_Platform_Functional_Specification_v0.6.md), [Embedded Plan v0.2](../Neuradix_Embedded_Profile_Implementation_Plan_v0.2.md), [Implementation Plan v0.4](../Neuradix_Implementation_Plan_v0.4.md)
- Integrated prototype crates: `neuradix-embedded-core`, `neuradix-embedded-codegen`, `neuradix-embedded-transport`

> Main includes the embedded/transport/codegen foundations and the PR #6 wire/ABI fixes. Host tests and actual AVR compile/link/rejection checks pass; physical board execution is still unverified. See [Capability Status](../Neuradix_Capability_Status.md).

## Implemented wire and ABI decision

The [Gate A implementation note](../implementation/Gate-A-Embedded-Wire-and-ABI.md)
records `neuradix.scalar-le.v2`: canonical name-sorted scalar offsets, a separate
full wire identity and generated decoder checks against the producer's binding.
The CLI emits a wire manifest alongside Rust/C++ source. `--cpp-target avr-uno`
rejects binary64; every C++ header checks the actual compiler ABI, including
portable headers compiled for AVR. This replaces the unversioned declaration-order
codec from development `c8aa467`.

Both endpoints and their channel bindings must be upgraded together. The serial
CRC/sequence frame format is unchanged and does not negotiate wire identity.
Compact-ID collision handling and legacy recording migration remain open.
The note preserves the exact tested commit, toolchain, memory measurements and
remaining WP-A01/A02/A03 acceptance work.

The earlier ESP32-C3 first-board choice remains historical; the current plan uses
Uno R3 plus one selected RP2040 board for physical Gate B acceptance.

## Problem

Constrained MCUs must participate in the same contracts, health, safety and
simulation ecosystem without running the full Linux runtime, and without leaking a
particular embedded executor into the SDK.

## Full target scope

`no_std` component API; static topology; bounded memory; health/identity; local
safe state; serial/CAN transport with framing/CRC/sequence; generated Rust
`no_std` and Arduino/C++ endpoints; host-simulation parity; and the
`neuradix embedded` CLI (`targets`/`new`/`check`/`generate`/`build`/`size`/
`flash`/`monitor`/`inspect`/`test`/`provision`/`update`).

## Proposed decision (intended)

- **Tiers**: Embedded Tiny (generated Arduino/AVR C/C++), Embedded MCU (native
  `no_std` Rust), Embedded Connected, Embedded High.
- **First targets**: host conformance, then actual Uno R3 and one 32-bit MCU as a paired Gate B milestone. RP2040 is the native planning default; further boards are maintained extension packs.
- **`embedded-core`**: executor-neutral static component trait, bounded ports,
  health, command lease, watchdog, deployment identity and safe-state interface.
- **Executor adapters**: host/static loop and one qualified native board executor first; Embassy/RTIC/other RTOS adapters follow a demonstrated need.
- **`embedded-codegen`**: `no_std` Rust, Arduino C++ and embedded C projections
  plus topology and memory-report generation, with golden encode/decode vectors.
- **Reference node**: an instrumented motor/sensor controller that validates local authority, enforces available measured limits and enters its declared state on link loss. Retain the AUV model as a regression/domain reference.
- Wireless links are never treated as a safety channel; safe state is local.

## Boundaries respected by increment 1

- `neuradix-contracts` and `neuradix-time` are already dependency-light and do not
  require the full runtime, so `embedded-core`/`embedded-codegen` can reuse the
  same contract model and clock-domain vocabulary (the plan's rule
  "`embedded-core → contracts/time, no full runtime dependency`").
- The Rust code generator is structured so additional target projections
  (`no_std` Rust, C++) are new emitters over the same validated `Contract`.

## Public interfaces and remaining extensions

The integrated `embedded-*` crates and `contract generate` will be extended by an `embedded` CLI subtree, reusing the same
application services and result schemas as the desktop CLI (Studio/CLI parity).

## Alternatives considered

- **A common embedded runtime shared with Linux.** Rejected: Arduino compatibility
  forces a *generated endpoint projection*, not a shared runtime implementation.
- **Leak Embassy/RTIC types into the SDK.** Rejected: `embedded-core` stays
  executor-neutral; adapters bind to a specific executor.

## Safety and security implications

Every actuator controller defines and enforces its local response to lease expiry and link/watchdog loss (NRX-EMB-006/020, NRX-PLAT-006/007). Gateway routing preserves identity/freshness; CRC is not authentication. Update verification/recovery must reflect actual board capabilities (NRX-PLAT-024).

## Compatibility implications

Contract projections require independent wire vectors and actual target ABI checks. Semantic identity cannot authorize an incompatible layout, and generated binary64 must not copy eight bytes through a four-byte Uno double. Target support levels and
conformance tests gate what "supported" means per board.

## Remaining conformance strategy

The embedded conformance suite (Implementation Plan ACC-02/03/04/05/06/12/14/16): encode/decode vectors,
timestamp/sequence handling, queue overflow, watchdog reset, lease expiry,
safe-state transition, health/identity, transport corruption detection, resource
budgets and host-simulation equivalence.

## Unresolved questions

- Confirm or record changes to the RP2040/Uno/serial planning defaults before dependent implementation.
- Static memory/timing budget expression in contracts.
- Deployment-identity representation in firmware.
