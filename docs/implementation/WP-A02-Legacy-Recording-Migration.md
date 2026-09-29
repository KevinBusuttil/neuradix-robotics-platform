# WP-A02.2: Legacy scalar recording migration

**Status:** the library increment is integrated through
[PR #31](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/31),
merge [`fdefa3d`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/fdefa3dda9911858240b29525f311cb91f9fb4e9)
(reviewed head `62a4b68`). CI passed on the head
([PR run 36557098614](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36557098614),
[push run 36557059733](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36557059733)),
and post-merge main CI passed all four jobs
([run 36557513776](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36557513776)).
The automated review of the head reported no findings.

The [`record migrate` CLI command](#cli-record-migrate-wp-a023) (WP-A02.3) is
integrated through [PR #32](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/32),
merge [`520c8e9`](https://github.com/KevinBusuttil/neuradix-robotics-platform/commit/520c8e959df5a2cd66e1c805f0c8226a2c4a2cfa)
(reviewed head `170c350`). CI passed on the head
([PR run 36560292725](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36560292725),
[push run 36560287804](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36560287804)),
and post-merge main CI passed all four jobs
([run 36564383664](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36564383664)).
The automated review found one P2 issue on the first head (a byte limit of
`usize::MAX` overflowed the `max + 1` sentinel), which was fixed in `170c350`;
its review of the final head reported no major issues.

This is host evidence only. It claims no board execution, Manufacturing
integration or Gate A closure, and WP-A02 remains open.

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

**Resources.** The library bounds its own inputs (provenance and contract
size, channel count). Its output memory is proportional to the recording it is
given.

*Correction:* this document previously said the existing readers already held
recordings in bounded memory. That was wrong for native recordings. The CLI
native loader (`record inspect`, `replay run`, `record export`) reads the whole
file with no size cap, and that is still true for those commands. Only MCAP
import is bounded there. `record migrate` (below) uses a new bounded native
reader.

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

## CLI: `record migrate` (WP-A02.3)

**Status:** integrated through PR #32 (see the status above).

```sh
neuradix [-o json] record migrate <input.nrec> --provenance <provenance.json> --out <output.nrec> \
  [--max-input-bytes N] [--max-records N]
```

The command is native to native. It calls `migrate_legacy_scalar` and
`LegacyProvenance::from_json` directly, and duplicates no decoding or
provenance rule. Every library guarantee above still holds:

- explicit provenance naming the pinned `c8aa467` `nostd-rust` producer;
- C++/AVR output rejected;
- strict payload checks;
- opaque channels preserved, including the fixture's −10 ns record;
- sequences, clock domains and recording provenance preserved;
- no inference from container version, missing wire metadata, schema identity
  or payload length.

| Aspect | Behaviour |
| --- | --- |
| Recording bound | `NativeRecording::from_reader` with `NativeReadLimits`: at most 256 MiB of container bytes and 1,048,576 records by default. `--max-input-bytes` and `--max-records` can only lower these. Bytes are read in 64 KiB chunks into a buffer whose capacity never exceeds the limit + 1 byte; the reader consumes at most limit + 1 bytes before rejecting. The record count is checked while decoding. |
| Provenance bound | Read through `take(1 MiB + 1)`, then rejected if longer. The read buffer may grow to about twice that. Each embedded contract is capped at 64 KiB by the library. |
| Memory | The read buffer is released after decoding and the source recording is dropped after conversion. Output is streamed to disk, not assembled in memory. At peak the process holds the decoded source and the converted copy, because the library copies opaque payloads too. **Measured** peak RSS with the release build: 484 MiB for a 240 MiB input of 4 opaque 60 MiB payloads (≈2.0×); 228 MiB for a 47 MiB input of 1,048,000 16-byte legacy records (≈4.7×, dominated by per-record overhead of roughly 90 B per record per copy). The worst case at the default limits is therefore about 512 MiB (byte-dominated) or roughly 230 MiB plus payload (record-dominated). |
| Source safety | The input is only read. A destination that already exists is refused (exit 2), including a symlink, a dangling symlink and the provenance file. When it resolves to the input's device and inode (the input path itself, a hard link or a symlink to it), the refusal names it as in-place migration. |
| Publication | Output goes to a new temporary sibling file, created with `create_new` in the destination directory. It is flushed and `fsync`ed, then published with `link(2)`, which fails with `AlreadyExists` instead of replacing a destination created concurrently. Removal of the temporary name is then attempted; if it fails, the command still succeeds and reports the leftover path as a warning. Success is reported only after publication. |
| Failure cleanup | Validation failures happen before any file is created. Write, `fsync` and link failures leave no destination, and removal of the temporary file is attempted. *Correction:* earlier text said the temporary file is always removed, but removal errors on these paths are ignored, so a failed removal can leave the temporary file. Since WP-A02.4 a failed removal after a successful publication is reported as a warning. Durability of the directory entry across power loss is **not** claimed (the directory is not `fsync`ed). Filesystems without hard-link support cannot publish; that fails cleanly. A process killed mid-write (for example by `SIGKILL`) can leave a `.<name>.neuradix-migrate-<pid>-<n>.partial` file, but never a partial destination. The publisher is shared with `channel manifest|table` (`crates/cli/src/app/publish.rs`). |
| Exit codes | 0 success; 2 invalid use (in-place, existing destination, out-of-range limits); 4 compatibility (provenance or payload rejected by the migration library); 1 general failure (I/O, missing or foreign input, an MCAP input, a truncated container, exceeded limits, a missing destination directory, publication). |
| Report (`data`) | `source`, `file`, `format`, `formatVersion`, `bytes`, `records`, `sourceCodec`, `sourceRevision`, `sourceDigest`, `migratedDigest`, `channels[{channelId, records, legacyWireLen, codecId, wireId}]`, `opaqueChannels` and `limits`. The two digests are reported separately and are not required to differ: migration can leave payload bytes unchanged, for example when legacy and canonical orders coincide. |

Evidence:

| Test | What it proves |
| --- | --- |
| `crates/cli/tests/migrate.rs` M1 | End to end on the authentic fixture. Exit 0, a JSON envelope with the pinned source and migrated digests, channel IDs, counts and wire IDs from `expected.json`. The published file is readable and its digest matches. Order, sequences, signed timestamps, domains, opaque bytes and provenance are preserved. The source is unchanged, no temporary file remains, and text output uses the same envelope. |
| M2 | Short revision, `cpp` generator, swapped field order, unknown field, unknown channel and malformed JSON each exit 4 and create nothing. A missing provenance file exits 1. |
| M3 | A truncated payload and a non-canonical `bool` exit 4, naming the record. Re-migrating a migrated recording exits 4 (already bound). |
| M4 | Input one byte over `--max-input-bytes`, and one record over `--max-records`, exit 1; exactly at both limits succeeds. Zero or raised limits exit 2. A 1 MiB + 2 byte provenance exits 1. |
| M5 | Input as its own destination, a hard link, a symlink to it, an existing file, a dangling symlink and the provenance file all exit 2 and are left untouched. |
| M6 | A missing destination directory, missing input, MCAP input and truncated container exit 1 and leave nothing behind. |
| Publication unit tests (`crates/cli/src/app/publish.rs` since WP-A02.4) | Publication: link then remove the temporary file; an injected write failure leaves no destination and no temporary file on a filesystem where removal succeeds; a destination created concurrently during the write is not replaced (`AlreadyExists`); a missing directory fails before writing. |
| `crates/record/tests/bounded_native.rs` | Exactly at the limits is accepted and one below is rejected. A 1 MiB stream against a 1 KiB cap consumes exactly 1,025 bytes. Chunked and `Interrupted` reads decode identically, including negative timestamps. Invalid limits and defaults are checked. |

## Remaining WP-A02 work

1. Channel-manifest emission and verification tooling: integrated in
   [WP-A02.4](WP-A02-Compact-Channel-Binding.md#channel-manifest-tooling-wp-a024)
   (PR #33).
2. Gateway integration of the compact-channel table: proposed in
   [WP-A02.5](WP-A02-Reference-Gateway.md) (not merged).
3. RFC-0024, recording the compact-ID, collision and migration rules
   normatively.
4. Link session binding (full manifest digest exchange, reconnect and reboot)
   in WP-B06.

Other recording commands (`record inspect`, `replay run`, `record export`)
still read native files unbounded; bounding them is separate work. Legacy
C++/AVR payloads remain unsupported by design, because their bytes depend on
the target ABI. WP-A02 and Gate A remain open.
