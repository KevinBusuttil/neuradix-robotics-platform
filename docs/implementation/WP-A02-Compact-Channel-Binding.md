# WP-A02.1: Verified compact channel binding

**Status:** integrated through
[PR #26](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/26),
merge [`d25344c`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/d25344ca8389e1de0bc2e864576c05cd1759bfe6)
(reviewed head `8ea8bef`). CI passed on the head:
[PR run 36525024688](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36525024688)
and [push run 36525001971](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36525001971).
Post-merge main CI also passed:
[run 36525272234](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36525272234).

**Post-merge defect (P1):** after the merge, an
[automated review finding](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/26#discussion_r4129868135)
showed that the merged `ChannelTable::new` accepted any digest with any valid
bindings. The [correction](#correction-digest-verified-tables) is integrated
through [PR #27](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/27),
merge [`df37afc`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/df37afc670f5f7d7e88f5f8ca96466b122254651)
(reviewed head `3fab90e`). CI passed on the head:
[PR run 36526605711](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36526605711)
and [push run 36526577509](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36526577509).
Post-merge main CI also passed:
[run 36528239753](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36528239753).
The automated review of the head left no findings. The defect was present on
`main` from `d25344c` until `df37afc`.

This is host and cross-compilation evidence only. It does not claim board
execution, a link session or Gate A closure.

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
| Digest | `ChannelManifest::digest` / `digest_preimage` | Format `neuradix.channel-manifest.v2` (the correction; v1 hashed JSON). SHA-256 of a length-prefixed binary preimage: `str(version) \| count u16`, then per entry in ascending compact-ID order `compact_id u16 \| wire_len u16 \| str(name) \| str(codec_id) \| str(schema_id) \| str(wire_id)`, where `str` is a `u16` LE byte length and UTF-8 bytes. Names are at most 128 bytes. The digest names a manifest; it does not authenticate one. |
| Receiver check | `ChannelManifest::verify_layout` | Resolves a compact ID and requires the entry to carry exactly the layout the receiver's generated decoder was built from. Equal length is not enough. |
| Envelope | `crates/embedded-transport/src/channel.rs` (no_std) | Sits inside the unchanged CRC frame payload as `'N' \| version 1 \| compact_id u16 LE \| manifest_tag [4] \| body`. The manifest tag is the first four bytes of the manifest digest. |
| Board table source | `neuradix_embedded_codegen::generate_channel_table` (correction) | Emits `MANIFEST_DIGEST` and `CHANNELS` (complete entries, compact-ID order) from one verified manifest, so bindings are no longer hand-copied. Golden: `crates/embedded-codegen/tests/golden/channel_table.rs`. |
| Channel table | `ChannelTable<N>::new(digest, bindings)` | Fixed capacity, no allocation. Rejects the whole table for any of: <ul><li>a reserved ID, a collision, or bindings not in ascending compact-ID order;</li><li>a codec other than `neuradix.scalar-le.v2`;</li><li>an empty, overlong, control-character or duplicate name;</li><li>a malformed schema or wire ID;</li><li>a bad length, overflow or an empty list;</li><li>(correction) a supplied digest that is not the SHA-256 of these exact bindings, recomputed without allocation by `manifest_digest`.</li></ul> A gateway never runs on a partial, ambiguous or mismatched mapping. |
| Resolution | `ChannelTable::open` | Rejects all of the following before any decoder runs: <ul><li>a truncated header;</li><li>a wrong magic byte (for example, an unenveloped legacy or command payload);</li><li>an unknown version;</li><li>a manifest tag that differs from the table's;</li><li>an unknown or reserved compact ID;</li><li>a body length other than the bound length.</li></ul> The resolved `binding.wire_id` is the peer wire ID for the generated `decode(body, peer_wire_id)`. |
| Sender | `ChannelTable::seal` | Refuses an unbound channel, a body whose length differs from the bound length, and a short buffer. |

Compact IDs only select an entry. Identity is still enforced in two places:

1. The manifest tag rejects a peer provisioned from a different manifest. This
   catches a remapped compact ID.
2. The generated decoder rejects any wire ID other than its own. This catches
   an equal-length foreign layout on a correctly resolved channel.

## Correction: digest-verified tables

*Integrated in PR #27 (`df37afc`).*

**Root cause.** In PR #26, `ChannelTable::new(manifest_digest, bindings)`
validated each binding but stored the digest without checking it. A board
could be given manifest A's digest with manifest B's bindings. Envelopes from
an A sender then passed the tag check and resolved through B. When a compact
ID and length coincided, the generated decoder received B's wire ID and
decoded A's bytes. The bindings also lacked the name, codec and schema fields,
so a board could not recompute the digest at all.

**Reproduction.** A regression was run before the fix on `d25344c`.
`crates/embedded-codegen/tests/channel_binding.rs` built A with `range` on
compact 1 and B with `depth` on compact 1 (both 16 bytes). The mixed table was
accepted, and the A sender's range bytes decoded as:

```text
inconsistent table accepted; A-sender range bytes decode as
Ok(Some(VehicleDepth { depth: 0.0004767922794117647, uncertainty: 0.0004767922794117647 }))
```

**Correction.** The digest and the exact mapping are now verified together
inside the only public constructor:

- **Complete entries.** `ChannelBinding` now carries every hashed manifest
  field: `compact_id`, `wire_len`, `name`, `codec_id`, `schema_id` and
  `wire_id`.
- **Binary preimage.** The manifest digest moves to a binary preimage
  (format v2) that the board can recompute.
- **Recomputation.** `ChannelTable::new` checks every binding, requires
  ascending compact-ID order, recomputes SHA-256 over the bindings with a
  streaming, allocation-free `sha2` (no default features), and returns
  `BindError::DigestMismatch` unless it equals the supplied digest. No table
  exists, so no frame reaches a decoder.
- **Generated board table.** `generate_channel_table` emits the digest and
  bindings from one verified manifest. Hand-editing either constant makes
  construction fail.

This is the smallest sound shape. A wrapper that stored the digest and
bindings side by side would still accept an inconsistent pair. Generated
constants alone would not stop hand-assembled tables.

**What this proves and what it does not.** The check proves that a table is
self-consistent: its digest is the digest of its own mapping. It does not
prove which manifest a peer holds, and it does not authenticate anyone. The
four-byte tag remains a weak mismatch detector. A consistent table for the
wrong manifest is still possible, and is caught only by the tag (with
probability 1 − 2⁻³²) or by a later session. Full digest exchange and session
identity remain in WP-B06.

**API and format migration.**

| Change | Migration |
| --- | --- |
| `CHANNEL_MANIFEST_VERSION` is `neuradix.channel-manifest.v2` | `parse` rejects v1 documents with `UnsupportedVersion`. Re-emit the same entries with `ChannelManifest::new(...).to_json_pretty()`. No v1 manifest was persisted by repository tooling. |
| v1 tags and v2 tags differ | An endpoint provisioned under v1 is refused by a v2 endpoint's tag check. Upgrade both endpoints together. |
| New host API | `ChannelManifest::digest_preimage`; names are at most 128 bytes. |
| `ChannelBinding` has new fields: `name`, `codec_id`, `schema_id` | Generate the bindings with `generate_channel_table`. |
| `ChannelTable::new` rejects more | It now also returns `DigestMismatch`, `Unordered`, `UnsupportedCodec`, `InvalidName` and `MalformedSchemaId`. |
| New transport API | `manifest_digest(&[ChannelBinding])`. |

The envelope format is unchanged: magic, version 1 and the 8-byte header stay
as they were. Existing codec goldens and wire IDs are unchanged.

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
| `crates/contracts/tests/channel_manifest.rs` M1–M7 | <ul><li>Round trip, and input-order independence.</li><li>Collisions, including identical identities, are rejected.</li><li>Every entry rule is enforced, including v1 and legacy codec labels and truncated, uppercase or overlong IDs.</li><li>Tampering is detected: rebinding an entry, swapping IDs or removing the digest is rejected, as are unknown fields, a wrong version and a colliding edit.</li><li>`verify_layout` rejects an equal-length foreign layout.</li><li>The v2 binary preimage and one digest are pinned (M6).</li><li>v1 documents are rejected, not reinterpreted, and re-emitting the same entries is the migration (M7).</li><li>An overlong name is rejected.</li></ul> |
| `crates/embedded-transport/tests/channel.rs` T1–T6 | <ul><li>Envelope through a CRC frame and `FrameDecoder`, using tables built from the host implementation's digest.</li><li>Whole-table rejection for a collision (including after valid bindings), unordered bindings, a reserved ID, malformed schema and wire IDs, v1, legacy and empty codecs, invalid and duplicate names, bad lengths, capacity and an empty list.</li><li>T6 (P1): the board and host digests are equal; another manifest's digest, an arbitrary digest, an edit to any hashed field, a subset or a superset each return `DigestMismatch`.</li><li>Every `open` rejection path.</li><li>An 87-byte command payload is not an envelope.</li><li>Every `seal` rejection path, and a cross-manifest rejection.</li></ul> |
| `crates/embedded-codegen/tests/channel_binding.rs` E1–E7 | End to end through a manifest, a table, a frame and the checked-in generated `VehicleDepth` decoder: <ul><li>a bound channel decodes;</li><li>an equal-length foreign channel resolves but does not decode;</li><li>a remapped producer is rejected by the manifest tag;</li><li>collision and v1-codec manifests are rejected;</li><li>an unenveloped legacy frame is rejected;</li><li>E6 (P1 regression): A's digest with B's bindings returns `DigestMismatch`, and the consistent B table refuses the A sender by tag;</li><li>E7: the checked-in generated board table verifies and decodes, its foreign channel does not decode as `VehicleDepth`, and an edited binding or digest is refused.</li></ul> Mutation check: with the digest comparison disabled, E6 and E7 fail. |
| `crates/embedded-codegen/tests/golden.rs` | The generated board table matches `tests/golden/channel_table.rs`; the existing projection goldens are unchanged. |
| `tools/ci/mcu_actuator.sh` | Builds `neuradix-embedded-transport` in release for thumbv6m, thumbv7em and riscv32imc. It includes the transport and `sha2` in the allocator-symbol scan, and (correction) fails if `channel::manifest_digest` is missing from the transport objects. On thumbv6m that function is 1,748 bytes. Neither crate has `alloc`. |

CI runs the three suites in a named step, "Compact channel binding
regressions", as well as in the workspace test step.

Local validation of the correction, all passing:
- `cargo fmt --all -- --check`
- `cargo clippy --locked --workspace --all-targets -- -D warnings`
- `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps`
- the channel suites: 7 manifest + 6 envelope + 7 end-to-end tests, and 2 golden tests
- `cargo test --locked -p neuradix-embedded-transport`: 21 tests and the doctest
- every `--no-default-features` check in CI, including `neuradix-command-core --features provisioning`
- `tools/ci/mcu_actuator.sh` on all three targets
- AVR: `cargo test -p neuradix-embedded-codegen --test avr -- --ignored`, 2 passed with avr-gcc
- reservation regressions and the release fault campaign (`reservation_faults -- --ignored`, 3 passed)
- `cargo test --locked --workspace` and the Python unit tests, run as an unprivileged user

Environment notes:
- Run as root, the workspace suite fails in `neuradix-python`, whose launcher
  deliberately refuses privileged execution.
- A root-owned temporary directory left by that run made one CLI test fail
  under the unprivileged user. After removing the directory, that suite
  passed (8 tests).

The MCAP fixture steps were not rerun locally; this change does not touch
`neuradix-record`.

## Channel-manifest tooling (WP-A02.4)

**Status:** integrated through
[PR #33](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/33),
merge [`981e750`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/981e7509efa1bc2801aaef5d125fdda914bdc422)
(reviewed head `5766f23`). CI passed on the head:
[PR run 36571324713](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36571324713)
and [push run 36571315761](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36571315761).
Post-merge main CI also passed all four jobs:
[run 36572617715](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36572617715).
The automated review's P2 finding was fixed on the head: a failure to check a
destination (for example a non-directory path component) now exits 1 rather
than 2. Its review of the final head reported no major issues.

```sh
neuradix [-o json] channel manifest <bindings.yaml> --out <manifest.json>
neuradix [-o json] channel verify <manifest.json> [--bindings <bindings.yaml>]
neuradix [-o json] channel table <bindings.yaml> --out <board_table.rs>
```

The binding specification assigns explicit compact IDs and names to authored
contracts. Contract paths are resolved relative to the specification:

```yaml
apiVersion: channels.neuradix.io/v1alpha1
kind: ChannelBindings
channels:
  - compactId: 1
    name: vehicle-depth
    contract: ../contracts/standard/navigation/vehicle-depth.yaml
  - compactId: 2
    name: tiny-telemetry
    contract: fixtures/tiny-telemetry.yaml
```

- **Identities are always derived.** The codec, schema ID, wire ID and wire
  length come from `WireLayout::for_contract` on each validated contract.
  Identity fields in a specification are unknown fields and are rejected.
- **Compact IDs are never derived.** They are never taken from hashes,
  assigned or renumbered. One contract can back several channels with
  different names and IDs.
- **The existing implementation is reused unchanged:**
  - entries: `ChannelEntry::for_layout`;
  - rules and digest: `ChannelManifest::new`, `parse`, `verify_layout` and
    `to_json_pretty` (format `neuradix.channel-manifest.v2`, with the same
    digest preimage and rules);
  - board table: `generate_channel_table`.

  There is no new digest algorithm, codec or manifest version.
- **Two verification levels.** They are reported separately:
  - `self-consistent`: the format, rules and declared digest check out. This
    comes with a warning, because anyone can recompute a valid digest for
    wrong bindings.
  - `matches-bindings`: additionally, every entry equals what the
    specification and its contracts produce. Missing, extra, renamed and
    mismatched entries are all rejected, including an equal-width foreign
    layout whose digest is correct.

  Neither level authenticates a peer or establishes a link session.

| Aspect | Behaviour |
| --- | --- |
| Limits | Each is enforced while reading (`take(limit + 1)`): binding specification 64 KiB; manifest 1 MiB; each contract 64 KiB; distinct contracts 4 MiB in total; at most 256 channels. The channel count is checked before any contract is read. |
| Outputs | `--out` is always the artifact; the global `--output` still selects the report format. An existing destination is refused with exit 2, including the specification, a contract being read, or a symlink to either. Publication uses the shared no-overwrite publisher (`crates/cli/src/app/publish.rs`): a `create_new` temporary sibling is `fsync`ed and published with `link(2)`, so a concurrently created destination is not replaced. Removal of the temporary file is attempted on every path. A failed removal after publication is reported as a warning. On error paths, or if the process is killed, a `.partial` file can remain, but never a partial destination. The directory is not `fsync`ed, and power-loss durability is not claimed. |
| Exit codes | 0 success (after publication); 2 existing destination or input; 3 contract parse, validation or unsupported (for example `string`) field; 4 binding specification or manifest rule violation, self-consistency failure, or mismatch with the bindings; 1 I/O, missing files, exceeded byte limits, missing destination directory, publication. |
| Reports | `digest`, `channels[{compactId, name, codecId, schemaId, wireId, wireLen}]`, `file`, `bytes` and `limits`. `manifest` adds `manifestVersion` and `contractBytes`. `verify` adds `level`, `selfConsistent`, `matchesBindings`, `expectedDigest` and `mismatches`. |

Evidence (`crates/cli/tests/channel.rs`):

| Test | What it proves |
| --- | --- |
| C1 | The golden bindings (`crates/cli/tests/fixtures/channels/bindings.yaml`) produce the pinned golden digest `sha256:4a8c…97fb` and the existing `VehicleDepth` wire ID. The output round-trips through `parse` and `to_json_pretty`. Reordered binding entries and `vehicle-depth` with reversed field order give byte-identical output. One contract can back two named channels. |
| C2 | Self-consistent verification (with a warning) and bindings verification report distinct levels. |
| C3 | Four self-consistent manifests with valid digests are each rejected (exit 4) against the golden bindings: an equal-width foreign layout on channel 1, a missing channel, an extra channel, and a renamed channel. |
| C4 | A tampered digest, the v1 version, an unknown field, an uppercase wire ID, reserved ID 0, an ID collision and malformed JSON all fail self-consistency (exit 4). A missing manifest and one over 1 MiB exit 1. |
| C5 | Duplicate or reserved IDs, duplicate names, supplied identity fields, a wrong `apiVersion` and 257 channels exit 4. A `string` field and invalid contract YAML exit 3. A missing contract or specification, oversized specification or contract, and more than 4 MiB of contracts in total exit 1. No output is created in any of these cases. |
| C6 | `channel table` emits source byte-identical to `crates/embedded-codegen/tests/golden/channel_table.rs`. `channel_binding.rs` E7 compiles that file, builds a `ChannelTable` from it, decodes through it, and rejects an edited binding or digest. |
| C7 | An existing file, the specification, and a symlink to a contract are refused (exit 2) and left untouched. A missing directory exits 1. Text output uses the same envelope. |

`crates/cli/src/app/publish.rs` unit tests cover publication, an injected
write failure, a concurrently created destination, a missing directory,
bounded reads and refused destinations. The no_std transport and MCU gates are
unchanged; this tooling runs on the host only, and no board execution is
claimed.

## Remaining work

WP-A02 is **not complete**:

1. **Existing-recording migration: done.** The legacy fixture, pinned
   decoder, conversion and recording wire metadata are integrated in
   [WP-A02.2](WP-A02-Legacy-Recording-Migration.md) (PR #31), and the bounded
   `record migrate` CLI in WP-A02.3 (PR #32).
2. **Tooling: done.** Manifest generation, verification and board-table
   emission from an explicit binding specification are integrated in
   [WP-A02.4](#channel-manifest-tooling-wp-a024) (PR #33). Deriving the
   binding specification from a deployment graph, including compact-ID
   allocation, is not included.
3. **Gateway integration.** A reference gateway that verifies the complete
   mapping before traffic, routes framed bytes to generated decoders and
   typed handlers, and reports the manifest digest with sourced identities is
   proposed in [WP-A02.5](WP-A02-Reference-Gateway.md) (not merged).
   Board-reported identity for NRX-EMB-005 still needs WP-B06 sessions and
   board execution.
4. **Session binding** (full digest exchange, reconnect and reboot), in WP-B06.
5. **RFC-0024**, recording the compact-ID, collision and migration rules
   normatively.

Gate A remains open. This increment does not change WP-A03 board execution,
WP-A04 or WP-A08.
