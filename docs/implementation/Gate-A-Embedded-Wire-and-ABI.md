# Gate A implementation 1: embedded wire identity and numeric ABI

Status: integrated into main through [PR #7](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/7), following
[PR #6](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/6). Code integration revision: `d5fbd69b7b901ad8899b9c911ad09e265ff76636`.
Scope: the first bounded increment of WP-A01, WP-A02 and WP-A03 in the
[Arduino-to-enterprise implementation plan](../Neuradix_Implementation_Plan_v0.4.md).
The broader strategy, specifications and capability register are maintained in
the [documentation index](../README.md).

## Baseline and purpose

This increment is based on development commit
[`c8aa467`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/c8aa4671bcee8739354beb7880551db7f64314fa),
on `claude/gifted-albattani-i7t2wc`. At the start of this work, main was at
`e39da5e`. PR #7 subsequently integrated the six development commits and these
codec fixes into main. The historical starting revisions remain here for
traceability. Integration does not close the remaining MCAP, safety, replay or
worker acceptance findings.

The shared engineering platform needs reliable contract exchange across Tiny,
MCU and host profiles before board deployment or distributed execution. Two
defects in the development generator prevented this:

1. Semantic schema identity sorts fields, but the embedded encoder used authored
   declaration order. Reordering two equal-width fields could silently swap
   values while retaining the same schema identity.
2. C++ mapped `float64` to `double` and copied eight bytes unconditionally.
   The classic Uno R3 ABI has a four-byte `double`; this could read or write past
   the field. Host compilation alone did not detect the mismatch.

## Implemented behavior

| Surface | Behavior in this increment |
| --- | --- |
| Semantic identity | Existing `schema_identity` and existing contract hashes remain unchanged. |
| Embedded wire layout | `neuradix.scalar-le.v2`: fields sorted by name, fixed little-endian scalars, no padding, exact payload length. |
| Wire identity | Full SHA-256 of an explicit descriptor containing codec version, schema identity, payload length and ordered field names/types/offsets/sizes. |
| Generated decoders | Require the sender's full wire identity; reject mismatched identity, wrong length and boolean bytes other than 0/1. C++ leaves the destination unchanged on rejection. |
| Numeric representation | Binary32 and binary64 preserve all bit patterns, including negative zero, subnormals, infinity and NaN payloads. No implicit narrowing. Finite-value policy remains a safety/contract concern. |
| C++ ABI | Headers check byte width and floating-point width/precision/exponent; use AVR-compatible C headers and C++11. Copies use the field's actual `sizeof` behind those assertions. |
| Uno selection | `--language cpp --cpp-target avr-uno` rejects `float64` before creating output. `portable` still validates the real compiler ABI at compile time. |
| CLI artifacts | Embedded generation writes source plus `<stem>.wire.json`; result JSON includes `wireId`, `codecId`, `wireManifest` and `cppTarget`. Host Rust generation remains a source projection without this embedded wire codec. |
| CI | Host C++ is required; a separate AVR job compiles and links supported scalars and requires a clear binary64 compile rejection. Rust checks use the existing pinned toolchain and locked dependencies. |

`avr-uno` describes numeric capabilities. It is not yet a board support pack,
Arduino sketch generator, scheduler, flasher or complete firmware application.
An RP2040 target and the remaining five-profile platform follow the master plan.
No AI service is required for this increment or its generated payload code.

## Try the first supported projection

With Rust 1.94.1 and a host C++ compiler installed, from the repository root:

```sh
cargo run --locked -p neuradix-cli -- contract generate \
  crates/embedded-codegen/tests/fixtures/tiny-telemetry.yaml \
  --language cpp --cpp-target avr-uno --out-dir target/tiny-codegen
```

This produces `tiny_telemetry.h` and `tiny_telemetry.wire.json`. The fixture
exercises boolean, binary32 and signed/unsigned 32/64-bit integers; it is a
conformance input, not a complete robot contract or hardware demonstration.

The standard `vehicle-depth.yaml` uses binary64, so selecting `avr-uno` for it
returns the contract-validation exit code 3. To use a binary32 depth contract,
author that type explicitly and accept its different schema identity, or add
an explicit conversion component. The generator never changes precision for you.

```sh
cargo test --locked -p neuradix-embedded-codegen
cargo test --locked -p neuradix-cli --test cli
# Requires gcc-avr, avr-libc and binutils-avr:
cargo test --locked -p neuradix-embedded-codegen --test avr -- --ignored --nocapture
```

The AVR tests are visibly ignored in the ordinary workspace suite and executed
explicitly by the mandatory AVR workflow job. A missing compiler fails that job.

## Wire binding and migration

`WireLayout::descriptor_bytes()` defines the exact compact JSON hash input.
The stable top-level key order is `codec_id`, `schema_id`, `wire_len`, `fields`;
each field uses `name`, `ty`, `offset`, `size`. Field order is lexicographic by
name. Hashing excludes `wire_id` itself. Tests pin the independent descriptor
for the existing vehicle-depth contract.

Generated Rust now uses `Payload::decode(input, peer_wire_id)`. C++ uses
`Payload::decode(input, length, output, peer_wire_id)`. Obtain `peer_wire_id`
from the producer's verified provisioning/channel manifest. Substituting the
receiver's own constant defeats the check. A wire hash identifies content;
it does not authenticate a peer.

The old codec was unversioned and order-dependent. Its payloads cannot safely
be identified from schema identity alone. Upgrade both endpoints and their
channel bindings together. Even where the old source already used sorted
fields and the bytes happen to match, do not relabel an unknown legacy stream
as v2. Old decoders cannot enforce these new checks.

For legacy recordings, retain the original ordered source/layout and codec
provenance, decode with that pinned legacy implementation, then explicitly
re-encode and record the new wire identity. If that provenance is missing,
reject automatic migration. A recording migration utility and recording-level
wire metadata are still open work under WP-A02/WP-A06; this change does not
rewrite existing recordings or their semantic schema IDs.

The serial framing crate is unchanged: its CRC and sequence number are not a
wire-identity handshake. Transport-level binding, collision-checked compact
channel IDs and their enforcement in gateways remain the next part of WP-A02.
No truncated hash is introduced here.

## Validation and work-package status

Code commit [`b05baed`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/b05baed366a91098b0bc0f0401c754d6974b6f46)
passed [CI run 34247995985](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34247995985)
on 2026-09-08. The subsequent documentation updates record these results without
changing the tested implementation. [Integration CI for PR #7](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34250744255) also passed both host and AVR jobs before merging the combined code into main.

| Check | Observed result |
| --- | --- |
| Rust 1.94.1, locked workspace dependencies | Format check, Clippy with warnings denied, workspace tests and documentation build passed. |
| Workspace tests including doctests | 191 passed, 0 failed; the 2 explicitly ignored AVR tests ran separately below. |
| Independent wire exchange | Generated C++ producer sent bytes plus its identity to independently generated Rust from a reordered contract; values and bytes matched. |
| Rejection and numeric boundaries | Mixed codec/schema identities, invalid lengths/booleans, signed/unsigned limits, binary32 special-value bits and CLI target rejection passed. |
| `no_std` source | Generated scalar library compiled without `std` using host `rustc`; this is not an MCU Rust target build. |
| AVR GCC 7.3.0 / ATmega328P | Both tests passed: supported scalar harness compiled/linked; portable binary64 header was rejected by the expected ABI assertion. |
| AVR harness static sections | `.text` 2,778 bytes, `.data` 322, `.bss` 6: 3,100 bytes of flash and 328 bytes of static SRAM. Stack and hardware timing are unmeasured. |
| Local C++ fixture | C++11, `-Wall -Wextra -Werror`, AddressSanitizer and UndefinedBehaviorSanitizer passed for binary64 bytes, identity/length rejection and special-value bits. Leak detection was disabled because this workspace runs under tracing. |

These results validate the codec increment. They do not validate a deployed
firmware image, real serial communication or physical board execution.

| Work package | This increment | Remaining acceptance work |
| --- | --- | --- |
| WP-A01 | Pins an existing development baseline; requires host C++ and a separate AVR compiler gate. | Complete evidence inventory and audit all optional-tool skips; the six development commits are integrated through PR #7. |
| WP-A02 | Canonical scalar layout, versioned full wire identity, required decoder identity and cross-language regression tests. | Gateway/transport binding, compact-ID collision enforcement, existing-recording migration fixture and tooling. |
| WP-A03 | Explicit AVR numeric profile, generated ABI guards, host scalar boundary tests and real AVR compile/link checks. | Execute golden vectors on a physical Uno, measure stack/runtime memory and timing, then add the selected MCU board profile. |

Gate A remains open. WP-A04 safety time/finite bounds, WP-A05 Python process
bounds, WP-A06 recording interoperability, WP-A07 actual program replay and
WP-A08 resolved deployment identity are unchanged by this increment. Board
execution, safety certification, Studio simulation workflows and enterprise
scaling are not claimed by these code-generation tests.
