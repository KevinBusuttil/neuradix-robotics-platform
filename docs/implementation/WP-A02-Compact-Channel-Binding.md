# WP-A02.1: Verified compact channel binding

**Status:** proposed for review. Not merged. This is host and cross-compilation
evidence only. It does not claim board execution, a link session or Gate A
closure.

## Scope

[Gate A wire evidence](Gate-A-Embedded-Wire-and-ABI.md) added canonical v2
scalar layouts, full wire identities and decoders that require the peer's wire
ID. It left three WP-A02 items open:

- transport binding;
- collision-checked compact channel IDs enforced in gateways;
- existing-recording migration.

This increment closes the first two for the serial envelope path. Recording
migration is still open (see [Remaining work](#remaining-work)).

## Design

| Part | Where | Rule |
| --- | --- | --- |
| Channel manifest | `crates/contracts/src/channel.rs` (host, std) | Explicit `u16` compact IDs are bound to full `codec_id`, `schema_id`, `wire_id` and `wire_len`. Compact IDs are never truncated hashes. |
| Manifest verification | `ChannelManifest::new` / `parse` | The whole manifest is rejected for any of the following: <ul><li>two entries share a compact ID, even with identical identities;</li><li>compact ID 0 (reserved);</li><li>a duplicate or invalid name;</li><li>a codec other than `neuradix.scalar-le.v2`;</li><li>a schema or wire ID that is not `sha256:` plus 64 lowercase hex digits;</li><li>a zero length, or one above `u16::MAX - 8`;</li><li>no entries;</li><li>unknown JSON fields or an unknown `manifest_version`;</li><li>a declared digest that differs from the recomputed one.</li></ul> One layout on two compact IDs is two channels, not a collision. |
| Digest | `ChannelManifest::digest` | SHA-256 of the compact JSON `{"manifest_version":"neuradix.channel-manifest.v1","channels":[…]}`. Entries are ordered by compact ID and their keys follow the struct order: `compact_id, name, codec_id, schema_id, wire_id, wire_len`. The digest names a manifest; it does not authenticate one. |
| Receiver check | `ChannelManifest::verify_layout` | Resolves a compact ID and requires the entry to carry exactly the layout the receiver's generated decoder was built from. Equal length is not enough. |
| Envelope | `crates/embedded-transport/src/channel.rs` (no_std) | Sits inside the unchanged CRC frame payload as `'N' \| version 1 \| compact_id u16 LE \| manifest_tag [4] \| body`. The manifest tag is the first four bytes of the manifest digest. |
| Channel table | `ChannelTable<N>::new(digest, bindings)` | Fixed capacity, no allocation. A reserved ID, a collision, a malformed wire ID, a bad length, overflow or an empty list rejects the whole table, so a gateway never runs on a partial or ambiguous mapping. |
| Resolution | `ChannelTable::open` | Rejects all of the following before any decoder runs: <ul><li>a truncated header;</li><li>a wrong magic byte (for example, an unenveloped legacy or command payload);</li><li>an unknown version;</li><li>a manifest tag that differs from the table's;</li><li>an unknown or reserved compact ID;</li><li>a body length other than the bound length.</li></ul> The resolved `binding.wire_id` is the peer wire ID for the generated `decode(body, peer_wire_id)`. |
| Sender | `ChannelTable::seal` | Refuses an unbound channel, a body whose length differs from the bound length, and a short buffer. |

Compact IDs only select an entry. Identity is still enforced in two places:

1. The manifest tag rejects a peer provisioned from a different manifest. This
   catches a remapped compact ID.
2. The generated decoder rejects any wire ID other than its own. This catches
   an equal-length foreign layout on a correctly resolved channel.

### What the envelope does not do

- **The manifest tag is a misconfiguration detector.** It is not an identity, a
  session or authentication. Two different manifests share a tag with
  probability 2⁻³², and anyone who can write the link can forge it.
- **Envelope mode is per link.** Magic and version bytes cannot prove that an
  arbitrary unenveloped legacy payload is not an envelope. Both endpoints and
  their manifests are upgraded together, as the Gate A note already requires.
- **Exchanging the full manifest digest at link start is session work.** That
  exchange, together with reconnect, reboot and replay handling, belongs to
  WP-B06, and the reserved compact ID 0 is kept free for it.
- **The A04.2 command payload is unchanged.** It is versioned and exact-length,
  and `open` rejects it by magic. Placing commands on a compact channel is left
  to the gateway integration that consumes this table.

## Evidence

| Test | What it proves |
| --- | --- |
| `crates/contracts/tests/channel_manifest.rs` M1–M6 | <ul><li>Round trip, and input-order independence.</li><li>Collisions, including identical identities, are rejected.</li><li>Every entry rule is enforced, including v1 and legacy codec labels and truncated, uppercase or overlong IDs.</li><li>Tampering is detected: rebinding an entry, swapping IDs or removing the digest is rejected, as are unknown fields, a wrong version and a colliding edit.</li><li>`verify_layout` rejects an equal-length foreign layout.</li><li>The digest hash input is pinned.</li></ul> |
| `crates/embedded-transport/tests/channel.rs` T1–T5 | <ul><li>Envelope through a CRC frame and `FrameDecoder`.</li><li>Whole-table rejection for a collision (including after valid bindings), a reserved ID, malformed wire IDs, bad lengths, capacity and an empty list.</li><li>Every `open` rejection path.</li><li>An 87-byte command payload is not an envelope.</li><li>Every `seal` rejection path, and a cross-manifest rejection.</li></ul> |
| `crates/embedded-codegen/tests/channel_binding.rs` E1–E5 | End to end through a manifest, a table, a frame and the checked-in generated `VehicleDepth` decoder: <ul><li>a bound channel decodes;</li><li>an equal-length foreign channel resolves but does not decode;</li><li>a remapped producer is rejected by the manifest tag;</li><li>collision and v1-codec manifests are rejected;</li><li>an unenveloped legacy frame is rejected.</li></ul> |
| `tools/ci/mcu_actuator.sh` | Now also builds `neuradix-embedded-transport` in release for thumbv6m, thumbv7em and riscv32imc, and includes it in the allocator-symbol scan. The crate is `no_std` without `alloc`, so it cannot allocate. |

CI runs the three suites in a named step, "Compact channel binding
regressions", as well as in the workspace test step.

Local validation:
- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets -- -D warnings`
- the three suites (6 + 5 + 5 tests)
- `cargo test --locked -p neuradix-embedded-transport` (all 20 tests plus the doctest)
- `cargo check --locked -p neuradix-embedded-transport --no-default-features`
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`
- `tools/ci/mcu_actuator.sh`

The codegen goldens and every existing wire ID are unchanged.

## Remaining work

WP-A02 is **not complete**:

1. **Existing-recording migration.** This needs three things:
   - a checked-in legacy fixture: a v1 declaration-order payload with its
     ordered source/layout and codec provenance from `c8aa467`;
   - a pinned legacy decoder that re-encodes to v2 and records the new wire
     identity, rejecting input without provenance;
   - optional `codec_id`/`wire_id` recording channel metadata, updated in step
     with the strict MCAP projection (`crates/record/src/mcap_import/projection.rs`).
2. **Tooling.**
   - A CLI command that emits and verifies channel manifests, for example from
     graph-resolved contracts.
   - Codegen that emits a board's `ChannelBinding` table from a verified
     manifest, so bindings are not hand-copied.
3. **Gateway integration.** An example or runtime gateway that routes opened
   envelopes to generated decoders. It must also report the manifest digest
   alongside firmware identity (NRX-EMB-005).
4. **Session binding** (full digest exchange, reconnect and reboot), in WP-B06.
5. **RFC-0024**, recording the compact-ID, collision and migration rules
   normatively.

Gate A remains open. This increment does not change WP-A03 board execution,
WP-A04 or WP-A08.
