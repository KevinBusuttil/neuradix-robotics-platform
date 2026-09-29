# WP-A02.2: Legacy scalar recording migration

**Status:** proposed for review, not merged. This is host evidence only. It
claims no board execution, Manufacturing integration or Gate A closure, and
WP-A02 remains open.

## Scope

WP-A02 acceptance requires "a migration fixture for existing recordings". The
[Gate A wire note](Gate-A-Embedded-Wire-and-ABI.md) set the rule: keep the
original ordered source and layout plus codec provenance, decode with a pinned
legacy implementation, re-encode explicitly, record the new wire identity, and
reject migration when that provenance is missing.

This increment delivers:

- a checked-in fixture produced by the historical code;
- a pinned legacy decoder;
- an explicit conversion to `neuradix.scalar-le.v2`;
- optional recording-level codec and wire metadata.

## The historical format

| Item | Finding |
| --- | --- |
| Producer | `c8aa4671bcee8739354beb7880551db7f64314fa` (2026-07-20). It is the only revision that generated the unversioned codec; `7a30679` introduced v2. |
| Layout | Scalar fields in **authored declaration order**, each fixed-width little-endian (IEEE-754 binary64/binary32, two's-complement integers, `bool` as one `0`/`1` byte). No header, no padding. |
| Historical decoder | Accepted trailing bytes (`input.len() >= WIRE_LEN`) and read any non-zero `bool` byte as `true`. It had no codec or wire identity. |
| C++ projection | Copied 8 bytes from a `double`. On AVR, `double` is 4 bytes, so those bytes are not a defined format. Only the `nostd-rust` producer is supported. |
| Container | The native container at that revision is `NRXREC`, `FORMAT_VERSION` 1, with the same channel shape (no wire field). The current writer reproduces the fixture's bytes exactly. |

`LEGACY_CODEC_LABEL`
(`neuradix.legacy.declaration-order-le@c8aa4671…`) is a descriptive label for
this pinned format. It never appeared on a wire.

## Fixture

`crates/record/tests/fixtures/legacy-scalar-c8aa467/` (see its
[README](../../crates/record/tests/fixtures/legacy-scalar-c8aa467/README.md)):

- **Reproducibility.** `generate.sh` builds the historical CLI and
  `neuradix-record` in a detached worktree at the pinned revision. It generates
  both structs and runs `producer.rs` against them. `generate.sh --check`
  reproduced the checked-in generated sources and recording byte for byte.
- **Recording contents.** Three channels:
  - a 45-byte mixed-width probe, where every field moves between legacy and v2
    order;
  - an opaque log channel, including a record at −10 ns;
  - `vehicle-depth` with its fields authored in reverse.
- **The equal-width hazard.** The reversed `vehicle-depth` channel has the same
  schema identity (`sha256:4c9c…2a73`) and the same 16-byte length as the
  current v2 contract. Legacy bytes read as v2 decode silently with depth and
  uncertainty swapped.
- **Expected values.** `expected.json` is written by hand from the producer's
  values. It pins the resulting wire IDs and both replay digests. The migrated
  depth channel's wire ID equals the existing `VehicleDepth` v2 golden wire ID,
  which confirms it independently.

## Migration API (`neuradix_record::legacy_scalar`)

| Step | Rule |
| --- | --- |
| Request | `LegacyProvenance` (`neuradix.legacy-scalar-provenance.v1`, unknown fields rejected, at most 1 MiB) names the producer revision and generator. For each channel it gives the recorded schema ID, the authored contract source (at most 64 KiB) and the declared field order. |
| Producer | The revision must be the full pinned hash; abbreviations and other revisions are rejected. The generator must be `nostd-rust`. |
| Channel | Must exist and must record no wire metadata (`AlreadyBound` otherwise). Duplicate, unknown or empty channel lists are rejected, and at most 256 channels are allowed. |
| Consistency | The recording's schema ID, the provenance schema ID and `schema_identity(contract)` must all agree. The declared field order must equal the contract's authored order (names and types). String fields are rejected. |
| Payload | Every payload must be exactly the legacy length, and every `bool` byte must be `0` or `1`. Payloads the historical decoder accepted but its encoder could not have produced are rejected, not normalized. |
| Conversion | Each field's bytes are copied from their legacy offset to their canonical offset, preserving values bit for bit (including `-0.0`). |
| Output | Channels not named in the request are copied unchanged as opaque bytes. Channel IDs, names, schema IDs, clock domains, sequences, timestamps, record order, writer, seed, note and software entries are preserved. Migrated channels gain `wire = {codec_id, wire_id}`, and one software entry `neuradix-record/legacy-scalar-migration` records the source revision. |
| Report | `MigrationReport` gives the source label and revision, source and migrated replay digests, and per-channel counts, legacy length, codec and wire ID. |
| Atomicity | Every check runs before output; any failure produces nothing. |

No migration is ever inferred. A native `format_version` 1 recording, or a
channel without wire metadata, is not assumed to hold legacy scalar payloads.
Container version, scalar codec version and channel-manifest version stay
independent. Field order is never guessed from schema identity or payload
length.

**Resources.** Bounded inputs (provenance and contract size, channel count).
Output memory is proportional to the input recording, which the existing
bounded readers already hold. No new unbounded reader is introduced.

## Recording wire metadata and compatibility

| Surface | Behaviour |
| --- | --- |
| `Channel.wire: Option<ChannelWire>` | Serialized only when present (`skip_serializing_if`), and inner unknown fields are rejected. Manifests without it serialize exactly as before: the current writer reproduces the c8aa467 fixture bytes. Absence means "not recorded", never "legacy". |
| Native container | No format change: `FORMAT_VERSION` stays 1 because the field is additive (RFC-0015). Older readers ignore it and keep treating payloads as opaque. |
| Replay digest | Unchanged definition. The manifest is excluded, so wire metadata alone never changes a digest. Migration moves bytes, so the migrated digest differs; both are pinned and reported. Existing pinned digests are unaffected. |
| MCAP historical profile | Writes `codecId` and `wireId` channel metadata only when wire metadata is present, so existing outputs are unchanged. The strict projection now requires channel metadata to equal exactly what the writer would produce: 2 keys without wire metadata, 4 with it. A partial or mismatched binding is rejected. |
| MCAP times | Unsigned, as before. The fixture's −10 ns opaque record is native-only; the MCAP legs exclude it explicitly. |
| CLI `record inspect` | Adds `codecId`/`wireId` to a channel only when recorded (additive). |
| Source compatibility | `Channel` has a new public field, so struct literals need `wire: None` or `Channel::new`; the 16 in-repo literals were updated. `Channel::with_wire` and `ChannelWire::for_layout` are new. |

## Evidence

| Test | What it proves |
| --- | --- |
| `crates/record/tests/legacy_migration.rs` L1 | The fixture is container v1 with no wire metadata. Contract, provenance and recorded schema IDs agree, and the historical generated sources show the legacy offsets. Channels, sequences and timestamps match `expected.json`. |
| L2 | The fixture bytes equal an independent declaration-order encoding of the expected values, and the pinned decoder returns them in declaration order, bit for bit. |
| L3 | Migrated payloads equal an independent v2 encoding of the expected values. Opaque bytes, order, IDs, sequences, timestamps, domains and provenance are preserved, with one appended software entry. The pinned wire IDs and replay digests match. The result round-trips through the native container and the historical MCAP profile. |
| L4 | Reading legacy `vehicle-depth` bytes as v2 swaps the two values silently; migration does not. |
| L5 | Rejections: wrong provenance version; short, uppercase, empty or other revision; `cpp`/`rust`/`golden` generators; no channels; a duplicate or unknown channel; an opaque channel declared legacy; a contract from another channel; an edited or invalid contract source; swapped, missing or mistyped field order; oversize contract or document; unknown or missing JSON fields. |
| L6 | A string field and an already-bound channel are rejected. |
| L7 | Truncated, trailing, empty and non-canonical-`bool` payloads reject the whole request, reporting channel, sequence and record index. |
| L8 | A channel absent from the provenance keeps its bytes and gets no wire metadata. |
| L9 | No `wire` key without metadata. The current writer re-encodes the historical fixture byte for byte. Wire metadata leaves the digest unchanged, and unknown `wire` fields are rejected. |
| L10 | MCAP channel metadata has exactly 2 or 4 keys. A historical-profile archive whose `wireId` is missing is not projected. |
| `crates/cli/tests/record.rs` | `record inspect` shows `codecId`/`wireId` only for the migrated channel. |

Local validation, all passing:

- `generate.sh --check` against the historical revision;
- the `neuradix-record` suites (10 migration tests, plus the existing
  round-trip, import, MCAP and writer suites);
- `neuradix-cli --test record` (9);
- `generate.py --check`, `memory.py` and `verify_export.py` (the independent
  MCAP fixture hashes and memory evidence are unchanged);
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`
  and `cargo doc`.

The workspace and CI-equivalent results are recorded in the pull request.

## Remaining WP-A02 work

1. A CLI migration command (for example `neuradix record migrate --provenance`)
   and channel-manifest emission and verification tooling. The migration is a
   library API only in this increment.
2. Gateway integration of the compact-channel table (WP-A02.1).
3. RFC-0024, recording the compact-ID, collision and migration rules
   normatively.
4. Link session binding (full manifest digest exchange, reconnect and reboot)
   in WP-B06.

Legacy C++/AVR payloads remain unsupported by design; their bytes depend on the
target ABI. WP-A02 and Gate A remain open.
