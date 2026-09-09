# WP-A06 — Bounded MCAP import

Status: implemented on the A06 branch; integration and current-revision
verification are pending. WP-A06/ACC-08 remain partial. Gate A remains open.

## Baseline and defect

Current main was verified on GitHub as `741b5bc53ddf293c7d809497018dbc36f8a60b78`.
[PR #13](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/13)
was already merged, with successful
[post-merge CI 34327790906](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34327790906).
Only stale plan/register integration labels change here; A05 implementation is
unchanged. The isolated branch is `codex/a06-bounded-mcap-import`.
WP-A01 supplies the integrated source/compiler baseline prerequisite; its broader
evidence inventory and optional-tool audit remain partial.

The old MCAP reader ignored unknown opcodes, including chunks containing messages,
discarded full schema/encoding and publishing-time information, and read into
unbounded vectors. CLI loading first read the complete file. A successful return
could therefore omit external messages or misrepresent their timing.

## Parser and supported profile

Runtime dependencies are deliberately pinned in Cargo.toml/Cargo.lock:
`mcap = 0.25.0` with default features disabled, `lz4 = 1.28.1` and
`crc32fast = 1.5.0`. Existing locked dependency versions are unchanged. The LZ4
binding resolves to `lz4-sys 1.11.1+lz4-1.10.0`; it builds the native LZ4 library.
Recording remains a host crate; embedded no_std crates do not depend on it.

Primary sources reviewed:

- [MCAP specification](https://mcap.dev/spec): record layout, timestamps, chunks,
  metadata, summary sections and CRC conventions.
- [Pinned maintained parser](https://github.com/foxglove/mcap/tree/0e1528a3dad6942384dae40b64827567413a18cf/rust/mcap):
  sans-I/O `LinearReader` length limits, chunk-emission mode and CRC/EOF checks.
- [LZ4 frame specification](https://github.com/lz4/lz4/blob/dev/doc/lz4_Frame_format.md)
  and [Rust decoder API](https://docs.rs/lz4/1.28.1/lz4/struct.Decoder.html).
- [Independent Python producer](https://pypi.org/project/mcap/1.3.1/).

The maintained parser supplies record framing and semantic parsing. It does not
alone establish all required allocation bounds: its variable-length fields can
allocate from field counts, and automatic decompression is unsuitable for this
adapter's preallocation policy. An allocation-free preflight validates each
field/map/array extent before semantic parsing. Chunk-emission mode disables
automatic expansion. The adapter validates chunk envelopes, then decodes exactly
one preflighted LZ4 frame into a capped, pre-sized output and probes one extra
decoded byte. Actual length and nonzero CRC must agree before chunk events.
Decoded chunk contents are preflighted before their events; nested chunks reject.

Supported: uncompressed outer records, uncompressed chunks, and standard LZ4
frame chunks, including linked/independent blocks, optional content size and
checksums. LZ4 blocks are at most the format's 4 MiB maximum. Dictionary IDs,
skippable/concatenated frames, trailing frame bytes and reserved/version features
reject explicitly. Zstandard and other compression names reject.

Full schemas, opaque schema/message encodings, raw payloads, channel metadata,
named metadata, header profile/library, sequence and both timestamps are retained.
Unknown schema languages and message encodings require no decoder and stay
opaque; they are not silently discarded. Attachments, attachment indexes,
unknown/private opcodes, nested chunks and record extension bytes explicitly
return `UnsupportedMcap` or `UnsupportedMcapRecord`. There is no permissive skip
mode. Empty attachment-index summary groups contain no attachment data and are
accepted as structural offsets.

Definitions must precede use. Schema ID zero rejects; channel ID zero and schema
reference zero are valid. Identical repeated definitions/metadata are normalized;
conflicting IDs or named metadata reject. Duplicate map keys, malformed UTF-8,
truncation, overflow and dangling schema/channel references reject. The profile
requires a Header, DataEnd and Footer, valid section order, contiguous summary
groups, matching footer/group offsets, matching statistics and available CRCs.
One explicit compatibility exception admits the historical `neuradix-record/0.0.1`
writer's interleaved schema/channel summary repetitions, provided their definitions
match and no summary-offset records follow. That old writer layout is not claimed
as strict summary-group conformance; correcting writing remains a later increment.
Zero CRC means absent, not verified integrity. Known message/chunk/metadata indexes
and statistics retain their original bodies and ordinals. Index target offsets
are opaque audit data, never followed or certified; every data message is read
linearly. This is semantic preservation, not byte-identical container retention.

## Limits and ownership

`McapImportLimits` fields are private. Fallible builders accept the inclusive
ranges below; defaults are usable without unchecked construction. Every limit
applies independently. Equality passes; an increment beyond a budget fails.

| Limit | Default | Allowed range |
|---|---:|---:|
| Input bytes | 64 MiB | 32 B–1 TiB |
| Record body bytes, including compressed chunk body | 4 MiB | 32 B–64 MiB |
| Decoded bytes per chunk | 8 MiB | 1 B–64 MiB |
| Cumulative decoded chunk bytes | 256 MiB | 1 B–1 TiB |
| Message payload bytes | 1 MiB | 1 B–64 MiB |
| UTF-8 bytes per string | 64 KiB | 1 B–1 MiB |
| Records, counting chunk envelopes and inner records | 2,000,000 | 1–20,000,000 |
| Messages | 1,000,000 | 1–10,000,000 |
| Unique schemas | 4,096 | 1–65,535 |
| Unique channels | 4,096 | 1–65,536 |
| Entries per map/array | 4,096 | 1–65,536 |
| Accounted persistent definition bytes | 4 MiB | 1 KiB–256 MiB |
| Accounted archive bytes | 64 MiB | 1 KiB–1 GiB |

Lengths are checked before parser allocation and before chunk allocation.
Counters and offsets use checked arithmetic or are dominated by a validated
smaller count. The input consumes at most its budget plus one EOF probe byte.
Parser insertion requests are capped; actual reads are at most 64 KiB. Parser
record scratch and vector capacity can exceed a single read size, bounded by
the record cap. LZ4 has native bounded block/context scratch in addition to the
decoded chunk. The adapter does not advertise its logical counters as exact RSS.

Retained charges are wire body bytes + 512 bytes per semantic object + 128 bytes
per map/array entry, plus the copied metadata-name key, before copying into the archive. Definitions have a separate
persistent budget; named metadata consumes it too. The allowances cover object,
tree-node and vector-growth storage for the qualified host workload; allocator,
parser, temporary definition copies and native decoder overhead remain separate.
Configuration ceilings and count caps bound them, but RSS budgets are measured
workload/platform evidence, not a universal allocator guarantee.

`import_mcap(Read, limits, visitor)` owns one synchronous parse. The visitor
borrows payloads until return; no next event executes until then. There is no
channel, producer thread or hidden queue. Caller copies and callback latency are
caller-owned. `McapArchive::from_reader` adds bounded materialization; it returns
no partial archive after failure. Streaming events are provisional until complete
EOF/CRC/structure success; callers must commit their own side effects only after
success or roll them back. A callback error stops further reads immediately.

An arbitrary blocking `Read` or callback has no wall-time guarantee. Use this
regular-file import outside conventional local-control execution; externally
time supervised import jobs where required. This increment creates no background
work and grants recordings no command authority.

## Times and API migration

MCAP logging and publishing times remain separate `u64` nanoseconds, including
the full integer range. Their epoch/relationship is producer-defined. Original
clock metadata is retained verbatim; absence remains unknown. Header/profile
claims are not authentication. Arrival order or log time is not proof of source
freshness. No runtime evaluation time is derived from these values.

Use `McapArchive` for generic materialization and `import_mcap` for streaming.
The old `Recording`/`RawRecord` has one timestamp and a Neuradix manifest, so
`McapArchive::try_into_recording` explicitly validates the historical Neuradix
profile. It requires consistent schema identities, manifest/channel clock
metadata, supported manifest version/fields, equal publishing/logging times,
and no unrepresentable extra metadata/index data. Unknown profile/library
declarations reject this conversion; the historical `neuradix-record/0.0.1`
writer remains supported. JSON duplicate/unknown fields cannot silently vanish.
Projection has bounded temporary copies in addition to the archive budget.

`McapRecording::from_bytes` now applies default bounds and checked projection.
Use `from_reader(reader, limits)` to deliberately change policy. Existing valid
Neuradix MCAP output/manifest/digest behavior is preserved within those limits;
larger inputs must opt into larger validated limits or streaming. Native `.nrec`
and the buffered MCAP writer are unchanged. Writer streaming/limits remain open.

CLI MCAP loading uses a File instead of whole-file `read_to_end`. `record inspect`
supports generic archives, reports schema/encoding/metadata/raw-time semantics,
and returns a null legacy digest when conversion is unsupported. `replay run`,
`explain` and export require the checked legacy view and reject incompatible
archives. The minimal-depth-stream example demonstrates the new boundary.

## Verification

Current revision results and measured RSS are pending CI; do not infer a passing
release gate from source presence. Independent fixture generation/check passed
locally with the exact pinned Python packages. The local environment lacks
Rust/rustup/Cargo and AVR tools; pinned compilation, lint and conformance run in
GitHub CI. Failed intermediate checks are retained in branch history.

The required suites include externally timed adversarial children, exact limits,
unsupported records/compression, malformed lengths/maps, CRC/truncation, missing
and conflicting definitions, summary offsets, expansion beyond declarations,
callback backpressure, legacy compatibility and API migration. Independently
produced fixtures include complete schemas and unequal source/log timestamps.
See [fixture provenance and regeneration](../../crates/record/tests/fixtures/README.md).

Memory qualification uses independent Python-generated 8/128 MiB payload files,
64 KiB messages and approximately 1 MiB LZ4 chunks. Linux child RSS includes
importer/parser/decoder/allocator storage, excluding fixture generation. Declared
budgets are 64 MiB for streaming/budget rejection and 192 MiB for materialization;
streaming RSS growth is limited to 12 MiB across workloads. Each child has a
20-second external timeout, with an outer 90-second wrapper.

## Remaining acceptance

This increment supplies bounded import and independent producer evidence only.
WP-A06/ACC-08 remain partial pending bounded streaming writer work, supported
exports decoded by independent readers, and broader interchange/scale evidence.
ROS payload decoding, general replay execution, indexes for seeking, additional
compression, attachments and live recording are not implemented here.
WP-A05/ACC-09 remain partial for broader resource/platform requirements. WP-A04
remains partial, ACC-05 incomplete and Gate A open. No physical timing, board
execution or rig-safe-response evidence follows from compiler/host tests.
