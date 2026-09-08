---
title: "Neuradix Embedded — Implementation Plan"
author: "Busuttil Technologies Limited"
date: "8 September 2026"
version: "0.2 Draft"
status: "Current planning baseline; implementation status is separate"
supersedes: "Neuradix_Embedded_Profile_Implementation_Plan_v0.1.md"
---

Aligned to [Functional Specification v0.6](Neuradix_Robotics_Platform_Functional_Specification_v0.6.md) and [Detailed Implementation Plan v0.4](Neuradix_Implementation_Plan_v0.4.md). The master plan owns dependencies, estimates and release dates; [Capability Status](Neuradix_Capability_Status.md) owns current evidence.

# 1. Objective and release scope

Make Tiny and MCU first-class participants in the unified project/compiler, simulation, diagnostics and deployment workflow. The first cross-target gate requires an **actual Uno R3 and one 32-bit MCU** alongside Edge. Generated headers and host-only no_std tests are foundations; they do not satisfy physical-board acceptance.

Tiny uses generated bounded C/C++; MCU supports no_std Rust or qualified generated C/C++. Connected/High are MCU capability variants. Keep executor, HAL and transport types outside portable component interfaces. Rich schemas, registry resolution and heavyweight data processing remain on capable hosts.

# 2. Current baseline and remaining work

Main includes embedded core, serial framing and generated projections through PR #7. PR #6 adds canonical wire identity and target ABI validation with passing host and actual AVR compiler checks. See [Gate A implementation evidence](implementation/Gate-A-Embedded-Wire-and-ABI.md). Physical embedded execution is still unverified.

WP-A02 now has canonical v2 scalar layouts and full wire identities; transport binding, compact-ID collisions and recording migration remain. WP-A03 now rejects unsupported binary64 for the Uno and checks the actual compiler ABI; physical vectors and runtime memory/timing remain. WP-A04 must still align local authority and numeric limits; WP-B03 handles clocks/rollover/reboot. The `avr-uno` numeric profile is not a complete Arduino board package.

# 3. Board and executor policy

| Initial target | Implementation | Acceptance |
|---|---|---|
| Host conformance | Generated Rust/C++, protocol and static-loop fixtures | Independent encoders/decoders, incompatible-layout rejection and executable logic comparison |
| Uno R3 | Arduino CLI, generated static C++ topology | Actual build/flash/monitor, wire vectors, resources and local failure response |
| One RP2040 board | no_std Rust and one qualified static executor/HAL binding | Actual build/flash/monitor, scheduling/clock/watchdog evidence |
| Later board/RTOS packs | ESP32, STM32, nRF, Uno R4, Embassy/RTIC/RTOS as required | Separate maintainer, version matrix, hardware conformance and release tests |

RP2040 is a planning default, not a claim of support. A recorded target change must retain both Tiny and MCU coverage. Qualify one executor before multiplying adapters. The common project format must express target capabilities so board-specific code does not leak into portable interfaces.

# 4. Work package sequence

| Packages | Deliverables |
|---|---|
| A01–A04, A08 | Reviewed baseline; schema/layout identity; safe ABI generation; trusted authority; resolved graph semantics |
| B01/B02 | Project target/device models, immutable deployment lock and firmware generation inputs |
| B03 | Clock mapping with reboot epochs/rollover/uncertainty; units/frames; explicit rate semantics |
| B04/B05 | Native MCU and Uno packages: HAL/startup, fixed topology, watchdog, health and build/flash/monitor |
| B06/B07 | Serial session/gateway, bounded routing and executable Edge control/authority graph |
| C04/C05 | Unified Studio/CLI deployment and calibrated hardware-in-the-loop trials |
| E01/E04 | Compatible verified bundles, interruption recovery and release qualification |
| F03 | Additional supported board/protocol packs |

Detailed dependencies and engineer-week ranges are in [the master work packages](Neuradix_Implementation_Plan_v0.4.md#5-work-package-execution-contract).

# 5. Target package contract

Each package declares exact board/MCU revision; toolchain and build flags; supported numeric ABI and codecs; language/executor; HAL/peripheral bindings; firmware/contract/layout identity; static data and configured stack/task/buffer budgets; clocks and timer width; transport framing and maximum payload; boot/reset/watchdog behaviour; and update/security capabilities.

Compile-time reports must distinguish flash, static RAM and configured stack reserves. Hardware tests report observed stack margin and execution/response timing. The compiler rejects unsupported representations, allocations, peripherals and placement constraints. A target must not silently reduce binary64 precision or map an unavailable hardware capability to a dummy implementation.

Reserve application and driver memory as well as platform buffers on the Uno. Compact channel/schema IDs must resolve through a verified manifest with collision/session handling. Full metadata may stay on the host while the board reports the compact and firmware identities required to interpret data correctly.

# 6. Control, timing and transport rules

Apply authority using local trusted time and separately check source age, sequence, epoch and deadline. Handle timer rollover and reboot; a simulation pause must never suspend a physical safety timeout. Express slew/rate limits with a defined time basis. Validate finite values, configuration and final outputs.

The board implements its declared safe state independently of Studio, the gateway and enterprise services. Define that response using the actual rig and instruments. Current/thermal protections require suitable measurements and validation; a software field alone is insufficient evidence.

Start with bounded serial sessions. CRC covers transmission integrity, not authenticity. Enforce sequence/freshness, reconnect and duplicate handling. Authenticate capable network boundaries and document what the physical serial link trusts. CAN and other buses are separately tested packs; transport abstraction must not hide their latency/capacity differences.

For updates, verify target and integrity at the supported device/gateway boundary. Do not imply secure boot, onboard signature verification or dual-bank rollback on hardware lacking them. Wired recovery and explicit physical handling may be the qualified Tiny recovery path.

# 7. Conformance and release evidence

ACC-02/03/04/05/06/12/14/16 cover actual toolchains/boards, identity, resources, authority, HIL, disconnection, updates and support evidence. Include reordered fields, boundary values, invalid/unsupported numeric types, corrupt/truncated frames, duplicate/reordered messages, reconnect, clock rollover/reboot, gateway loss, watchdog and storage/queue pressure.

The same selected portable control logic runs in host simulation and on hardware with declared numeric/timing tolerance. This establishes interface/logic conformance, not complete physical fidelity. Record board, firmware and deployment revisions, test setup, measurements, pass/fail/not-run and reviewer.

Gate B requires actual Uno and MCU evidence. Gate C adds the integrated simulation/Studio/HIL workflow; E qualifies the named support matrix. No additional board becomes Supported merely because it shares a chip family.

# 8. Studio and CLI visibility

Expose target/firmware/deployment identity, contract/layout version, reset reason, watchdog state, clock mapping/uncertainty, deadline misses, queue overflows, link quality and local intervention. Show build flash/RAM budgets separately from measured runtime use. Flash/monitor and virtual/physical binding changes use shared application services, with clear active configuration and authority state.
