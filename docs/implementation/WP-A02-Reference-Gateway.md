# WP-A02.5: Reference gateway for verified compact channels

**Status:** proposed for review, not merged. Built on `main` at
[`981e750`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/981e7509efa1bc2801aaef5d125fdda914bdc422),
which integrates PR #33 (WP-A02.4).

This is host evidence only. It runs a simulated producer in the same process.
It does not execute on a board, open a serial port, establish a link session
or close WP-A02 or Gate A.

## Boundary

WP-A02 asks that compact endpoint IDs, collisions and mixed codecs can never
decode data silently. Earlier increments supplied the parts:
- the verified manifest and the no_std envelope and table (WP-A02.1, with its
  PR #27 correction);
- migration (WP-A02.2 and WP-A02.3);
- manifest and board-table tooling (WP-A02.4).

The remaining WP-A02 item was a gateway that uses those parts to route real
bytes to generated decoders and reports the manifest digest with the
available identities.

| In this increment | Deferred |
| --- | --- |
| Reusable routing library (`crates/gateway`, `neuradix-gateway`). It verifies the complete configured mapping before any traffic. | Live serial discovery and port handling (WP-B06). |
| A byte-stream gateway: incremental CRC frame decoding, envelope resolution, generated decoding, typed handlers. | Manifest exchange at link start, authentication, device-session negotiation (WP-B06). |
| A runnable host example with a simulated producer and two routes. | Reconnect and reboot handling, replay protection, duplicate suppression (WP-B06). |
| Resource bounds and a no-allocation check for sustained traffic. | Timing, stack and board resource measurement (B04/B05). |
| An identity report giving the full manifest digest and the source of each identity. | Firmware attestation. NRX-EMB-005 is not qualified on hardware. |
| Telemetry only. | Actuator or command dispatch. Command authority, freshness and watchdog behaviour are untouched. |

Also out of scope:
- RFC-0024;
- automatic graph-to-channel allocation;
- Manufacturing integration;
- hardware qualification;
- hardening of the other native recording readers.

## Data path

```text
generated encode ─▶ ChannelTable::seal ─▶ frame::encode           (simulated producer)
bytes ─▶ FrameDecoder<64> ─▶ ChannelTable::open ─▶ decode(body, binding.wire_id) ─▶ Handle<T>::handle
```

Every stage reuses existing code unchanged:
- `neuradix_embedded_transport::{FrameDecoder, encode, ChannelTable, SequenceTracker}`;
- `neuradix_contracts::{ChannelManifest::parse, verify_layout, WireLayout::for_contract}`;
- the board table emitted by `neuradix channel table`;
- decoders emitted by `neuradix contract generate --language nostd-rust`.

No codec, manifest version, envelope or frame format changes.

**Wire identity.** The generated decoder receives the **resolved binding's**
`wire_id`, never its own `WIRE_ID`. A unit test drives the real dispatch
function with a binding that carries another contract's wire ID and observes
that the decoder refuses it and no handler runs.

## Initialization: the complete mapping, before traffic

`RoutingTable::new(manifest_json, board_table, routes)` returns routing state
or an `InitError`. After an error no gateway exists, so no handler can run.

| Check | Rejects |
| --- | --- |
| Manifest artifact | Anything over 1 MiB (`MAX_MANIFEST_BYTES`). Anything that fails `ChannelManifest::parse`: malformed JSON, unknown fields or version, a rule violation, or a declared digest the entries do not produce (including another manifest's valid digest). |
| Capacity | More manifest channels or routes than the table capacity `N`. |
| Route list | Duplicate compact IDs or names. A contract the scalar codec cannot represent (for example `string`). An invalid contract document. |
| Route against manifest | A route with no manifest entry (a missing channel, or reserved ID 0). A name mismatch. An entry that does not carry the expected contract's layout (`verify_layout`); this catches an equal-width foreign contract with a valid digest. |
| Manifest against routes | A manifest entry with no route (an extra channel). |
| Decoder registrations | A generated decoder whose codec, schema ID, wire ID or length differs from the expected contract. |
| Compiled board table | `ChannelTable::new` failure: a digest paired with different bindings, or an edited binding or digest. A self-consistent table for a different manifest (`BoardTableMismatch`). |
| Frame buffer | `F` smaller than the largest bound envelope (`FrameCapacity`). |

The resulting state is immutable: there is no API to add, replace or remove
routes.

The four-byte envelope tag is still only a misconfiguration detector. Test R9
states the limit directly: a sender that copies the reference tag and puts
equal-width foreign bytes on channel 1 is decoded as `VehicleDepth`. Refusing
that needs the full manifest exchange and authentication of WP-B06.

## Input handling and resource limits

| Condition | Behaviour |
| --- | --- |
| Fragmented input | Bytes feed a fixed-buffer `FrameDecoder<F>` whose state persists across `push` calls. A frame split at any byte decodes identically. |
| Several frames per chunk | Each frame is routed as soon as it completes. |
| Incomplete frame | Held in the fixed buffer, and nothing is delivered. If it is never completed, the following bytes are read as its remainder, its CRC fails, and the frames that supplied those bytes are lost. `discard_partial()` drops it explicitly, for example after an idle gap. Detecting such gaps is WP-B06. |
| Oversized frame | A declared length above `F` is refused at the header, without buffering the payload, and the decoder resynchronises. It is reported as `CorruptFrame`, because the transport reports it the same way as a CRC failure. |
| CRC failure, noise | Dropped. Line noise is skipped while searching for the sync pattern. |
| Envelope | The following each produce a counted rejection and no handler call: truncated, wrong magic, wrong version, another manifest's tag, an unknown or reserved compact ID, a wrong body length. |
| Decoder | A body the generated decoder refuses (for example a non-canonical boolean) produces `Decode` and no handler call. |
| Sequence | Classified and counted: duplicates, missed frames and reordered frames. Not enforced; duplicated, reordered and gapped frames are delivered. |

**Bounds** (reference instance):
- frame buffer: 64 bytes (largest envelope 37 bytes);
- channel table: 8 entries;
- one route per bound channel;
- configured manifest: 1 MiB.

Initialization allocates on the host: the parsed manifest, the expected
layouts and the route list. Routing allocates nothing. There is no queue and
no per-frame output: handlers run synchronously. Counters saturate. No string
is leaked to obtain a `'static` lifetime; the bindings are the generated
board table's constants.

## Identity reporting

| Identity | Reference value | Source |
| --- | --- | --- |
| Manifest digest | `sha256:4a8cf8f6ba9d3479fad842b80b177e0d90b93bcf259f070fd37e4ac9a00d97fb` | Configured artifact (`crates/gateway/fixtures/reference-manifest.json`), verified equal to the compiled generated board table. |
| Node | `sim-telemetry-node` | Simulated fixture |
| Firmware/build | `sim-producer 0.0.1 (host simulation, not firmware)` | Simulated fixture |
| Deployment | `sha256:5a5a…5a` (placeholder, not a resolved deployment digest) | Simulated fixture |
| Gateway build | `neuradix-gateway 0.0.1` | Generated build information |

Every report states: *no peer attestation: identities are configured,
generated or simulated, not reported by a connected board.* Absent
identities are reported as `(not configured)`, never invented.
`embedded_core::DeploymentId` is reused. `NodeId` is not, because it requires
a `'static` name and host configuration would have to leak it.

## Running the example

```sh
cargo run -p neuradix-gateway --example reference_gateway
cargo run -p neuradix-gateway --example reference_gateway -- crates/gateway/fixtures/foreign-manifest.json   # exit 2
```

The run checks its own counts and exits 1 on any unexpected outcome. It
exits 2 when initialization refuses the configuration. An abridged run (wire IDs
shortened, rejection lines condensed to one per step):

```text
routes (verified against the manifest, contracts, decoders and board table)
  ch 1 vehicle-depth   -> VehicleDepth   wire sha256:4780f56b…d7b11d (16 B)
  ch 2 tiny-telemetry  -> TinyTelemetry  wire sha256:886b1941…e207a (29 B)
limits: frame buffer 64 B, largest envelope 37 B, table capacity 8

traffic
  two routes, two frames in one chunk  [1 chunk(s)]
    handler  vehicle-depth (ch 1): VehicleDepth { depth: 12.5, uncertainty: 0.25 }
    handler  tiny-telemetry (ch 2): TinyTelemetry { a_enabled: true, … }
  line noise, then one frame split into 3-byte fragments  [13 chunk(s)]
  CRC failure …                         rejected, no handler: corrupt frame
  oversized frame, then a valid frame   rejected; valid frame delivered
  unknown compact ID 9 and reserved ID 0     UnknownChannel(9), UnknownChannel(0)
  wrong length on ch 1 (15 bytes)            LengthMismatch
  producer provisioned from another manifest ManifestMismatch
  unenveloped payload                        BadMagic
  non-canonical bool on ch 2                 generated decoder refused the body
  incomplete frame, discarded; valid frame after the discard: delivered

summary
  bytes 575  frames 11  delivered 5 (vehicle-depth 2, tiny-telemetry 3)  rejected 8
  corrupt 2  unknown channel 2  length 1  manifest tag 1  bad magic 1  decoder 1
  sequence (observational, not enforced): missed 3  duplicates 0  reordered 0
result: all outcomes as expected
```

Tiny-telemetry is a codec conformance fixture. It serves here as a second,
differently shaped telemetry route, not as a board application.

## Evidence

Every test drives `neuradix-gateway` itself; there is no parallel test-only
router.

| Test | What it proves |
| --- | --- |
| `tests/generated.rs` G1–G3 | The checked-in decoders equal `generate_nostd_rust` output. The configured manifest equals `ChannelManifest::new(...).to_json_pretty()` with the pinned digest. The board table equals `generate_channel_table` output and the embedded-codegen golden. The foreign fixture is self-consistent and differs only in channel 1's identity. |
| `tests/init.rs` I1 | The reference configuration initialises. It reports the routes, the digest, the tag and the largest envelope. |
| I2 | A tampered digest, another manifest's valid digest, the v1 version, an unknown field, an uppercase identity, truncated JSON and a manifest over 1 MiB are all refused. |
| I3 | Self-consistent manifests that disagree with the expected routes are refused. The cases are: the equal-width foreign contract (the `channel manifest` fixture and a direct build), swapped IDs, swapped contracts, a renamed channel, a missing channel, and an extra channel carrying a contract that is already bound. |
| I4 | Duplicate IDs and names, a missing route, an extra route, a route for reserved ID 0, an unsupported (`string`) contract and an invalid contract are refused. |
| I5 | A decoder registered for the wrong contract, a decoder with the right wire ID but the wrong length, and a foreign expected contract are refused. |
| I6 | For the compiled board table, all of these are refused: a digest paired with a subset or an edited binding, a flipped digest, and a self-consistent table for another manifest. |
| I7 | Table and frame-buffer capacity are enforced. |
| `tests/routing.rs` R1 | Typed dispatch on both routes, with sequence classification and byte counts. |
| R2 | A two-frame stream split at every byte, one byte at a time, and in chunks of 2 to 13 bytes. |
| R3 | Fifty frames in one chunk, routed in order. |
| R4 | A bit flip in every byte after the sync pattern reaches no handler; the next valid frame is delivered. |
| R5 | Oversized frames (65 bytes and the largest declarable length) are dropped at the header, and the following frame is delivered. A buffer-sized non-channel payload is refused by length. |
| R6 | An incomplete frame delivers nothing, and completing it later delivers it. An abandoned partial frame loses the frame whose bytes completed it. `discard_partial` prevents that loss. |
| R7 | These are all refused: unknown IDs (9 and 65535), reserved ID 0, bodies that are too short or too long, a vehicle-depth body on the tiny-telemetry channel, a truncated envelope, wrong magic and version 2. |
| R8 | Equal-width foreign bytes tagged with the foreign manifest are refused by tag, and the gateway cannot be configured with that manifest. |
| R9 | The limit stated above: a copied tag is not authentication. |
| R10 | A non-canonical boolean is refused by the generated decoder, and no handler runs. |
| R11 | Duplicate, gapped and reordered frames are counted but delivered (deferred to WP-B06). |
| R12 | The identity report shows the full digest, the source of each identity, the limitation, and absent identities. |
| `tests/bounded.rs` B1 | 5,000 rounds of nine rejection kinds, line noise and both valid frames in 7-byte fragments. Counts are exact, the gateway's size is unchanged, and **no allocation** occurs while routing. |
| `src/decoder.rs` unit test | Dispatch passes the binding's wire ID: a foreign binding does not decode, and the matching binding does. |

**Mutation checks.** Each change was applied locally, one at a time, and
reverted. Each made at least one test fail:

| Mutation | Failing tests |
| --- | --- |
| Decoding with the decoder's own `WIRE_ID` | decoder unit test |
| Skipping `verify_layout` | I3, I5 |
| Skipping the decoder-registration check | I5 |
| Skipping the board-table comparison | I6 |
| Skipping the unrouted-channel check | I3, I4 |

CI runs `cargo test -p neuradix-gateway` and both example invocations in the
step "Reference gateway regressions and example"; the foreign-manifest run
must exit 2.

## What this satisfies, and what remains

This increment supplies the WP-A02 gateway-integration item. A gateway now
routes compact IDs only through a completely verified mapping. It refuses
collisions, missing, extra and mismatched routes and mixed identities before
any traffic. At runtime it refuses unknown, reserved, wrong-length,
wrong-manifest and undecodable frames without calling a handler. It also
reports the full digest with sourced identities.

WP-A02 stays **open** until this is reviewed and merged, and until RFC-0024
records the compact-ID, collision and migration rules normatively. Session
binding stays in WP-B06: full manifest exchange, authentication, reconnect and
reboot, replay and duplicate handling. The NRX-EMB-005 board-reported identity
also depends on WP-B06 and board execution. Gate A remains open.
