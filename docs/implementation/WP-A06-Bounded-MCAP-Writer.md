# WP-A06 — Bounded uncompressed MCAP streaming writer

Status: implemented on `codex/a06-bounded-mcap-writer`, [PR #15](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/15)
open and unmerged; integration pending.
Implementation verification passed as recorded below. WP-A06/ACC-08 remain
partial and Gate A stays open.

## Baseline and prerequisite review

GitHub main was verified as `79286d1e16280fc3ecadb7fa94dc3e8f98af9fa3`.
[PR #14](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/14)
was already merged, with unchanged reviewed head
`7b73c9434c36e68f146bc1251bd2e8586e0b21d4` and successful
[post-merge CI 34334529747](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34334529747).
Its PR/push CI `34334036127`/`34334031890` also passed. Main was unprotected,
rulesets were empty and no required external review blocked this branch.
The isolated checkout preserves previous branches and work. PR #13 integration
and WP-A05 implementation are unchanged.

WP-A01 supplies the integrated source/compiler baseline through PR #7. Its broader
capability/evidence inventory and optional-tool audit remain partial; that work is
not relabelled complete by this increment.

PR #14 review follow-ups included here:

- CLI inspection rescanned every message per channel, and legacy projection
  linearly searched the manifest and reparsed domains per message. Import now
  exposes validated per-channel counts; projection indexes domains once. Lookup
  work is O(log channels) per message and inspection uses the accumulated counts.
- A Statistics map could omit a channel carrying messages while reporting a
  correct total. Every observed nonzero channel must now be present and agree.
  Omission of a zero-count channel remains valid.
- Duplicate-map overwrite was a review concern, but pinned parser source already
  rejects duplicate string/integer keys. New malformed-map regressions exercise
  that rejection without duplicating the parser's implementation.

## Defect and maintained implementation

The old writer copied every payload, then constructed another complete file on
finish. Memory grew with recording length; writes were invisible before finish.
Its summary interleaved schema/channel repetitions, and it invented undeclared
channels or allowed a sample to override the manifest's clock domain.
CLI export added another whole-output Vec.

`McapStreamWriter<W: Write>` uses the existing exact `mcap = 0.25.0` dependency,
with default features disabled. No dependency or Cargo.lock additions are needed.
Primary behavior was checked in the
[pinned writer source](https://github.com/foxglove/mcap/blob/0e1528a3dad6942384dae40b64827567413a18cf/rust/mcap/src/write.rs),
the [MCAP specification](https://mcap.dev/spec) and the independent
[Python StreamReader API](https://mcap.dev/docs/python/mcap-apidoc/mcap.stream_reader.html).

The adapter disables chunks, compression, seeking, message/chunk/metadata indexes,
repeated definitions and summary offsets, then enables Statistics and data/summary
CRCs. `NoSeek` tracks position over a Write sink. No payload queue, compression
thread or background task is created. Direct messages borrow their bytes until the
synchronous call returns; the sink supplies backpressure. Definition serialization
uses bounded upstream scratch and finalization makes bounded definition copies.
The word “streaming” alone is not an allocation or wall-time guarantee.

## Limits, accounting and ownership

`McapWriteLimits` fields are private. Fallible builders validate inclusive ranges;
constructor admission also checks that the selected limits can hold the header
and finalization. Limits apply independently; no unvalidated mutation is exposed.

| Limit | Default | Allowed range |
|---|---:|---:|
| Complete file bytes | 64 MiB | 32 B–1 TiB |
| Record body bytes | 4 MiB | 32 B–64 MiB |
| Message payload bytes | 1 MiB | 1 B–64 MiB |
| UTF-8 bytes per string | 64 KiB | 1 B–1 MiB |
| Physical records, including finalization | 2,000,000 | 1–20,000,000 |
| Messages | 1,000,000 | 1–10,000,000 |
| Unique schema IDs | 4,096 | 1–65,535 |
| Unique channel IDs | 4,096 | 1–65,536 |
| Unique metadata names | 4,096 | 1–65,536 |
| Entries per map, including channel statistics | 4,096 | 1–65,536 |
| Accounted definition/finalization storage | 4 MiB | 1 KiB–256 MiB |

Admission computes wire sizes without serialization/allocation, checking arithmetic
and encoded u32 lengths before calling the maintained writer. The sink independently
enforces the total byte cap. Counters either use checked arithmetic or are dominated
by smaller validated ceilings; no counter can approach its machine maximum.
Every accepted record reserves its framing plus finalization. Finalization needs
`105 + 10 × active_channels` bytes and three records: DataEnd, Statistics, Footer,
plus trailing magic. Statistics body size `46 + 10 × active_channels` also obeys
the record cap, and its entry count obeys the map cap.

Retained state charges the header and each new definition/metadata name by
`4 × wire_body_bytes + 2048 + 256 × map_entries`. This covers canonical definitions,
ID maps, counts, named metadata copies and bounded finalization copies for the
qualified workload. Upstream serialization Vec capacity, allocator overhead and
caller-owned inputs/sinks remain separately bounded scratch or external ownership;
the charge is not an exact allocator/RSS meter. The legacy facade counts manifest
JSON through a bounded counting sink before allocating it and reserves its metadata
admission first. Temporary legacy definition copies are bounded by that admitted
manifest. Its domain map is bounded by the admitted channel count.

Messages add no retained payload and no per-message index. Duplicate identical
definitions/metadata normalize without another physical record; conflicts fail.
Metadata names and unique IDs cannot grow beyond count and state caps. Caller
`Vec` sinks and caller copies can grow; regular File sinks are used for memory
qualification. Recording length may increase until its configured output/count cap,
while writer state stays fixed for a fixed admitted definition set.

## Lifecycle, structure and integrity

Initialization validates the policy/header and reserves the empty tail before
writing magic/Header. The sink must be a fresh byte stream at offset zero;
Write alone cannot inspect pre-existing content. Appending a session to an existing
MCAP file is unsupported. Every write, admission or flush error latches failure. Later
operations return `McapWriterFailed` without writing. Finish consumes the writer,
writes the reserved tail and flushes; any failure returns an error, with no retry
that could report success. The adapter explicitly suppresses upstream automatic
Drop finalization using `into_inner`; abort returns the provisional sink without
a footer. Drop does not attempt to repair a failed prefix.

Prefixes are provisional, including complete-looking bytes after a flush failure.
Only successful explicit finish permits publication. Generic callers must discard
failed/aborted files. CLI export uses a create-new temporary sibling and renames it
only after finish; error paths close/delete the temporary file and preserve an
existing destination. A crash may leave a `.partial` file for external cleanup.
Rename replacement behavior is OS/filesystem specific; failure is reported.
Flush/rename do not promise fsync or power-loss durability. Arbitrarily blocking
Write/flush/filesystems need external supervision; there is no wall-time guarantee.

Data definitions precede references. The summary is one contiguous Statistics
group; complete schemas/channels remain in the data section. Footer summary_start
points to that group, and summary_offset_start is zero. Sequential unindexed
reading is required; no seeking indexes are advertised. Data CRC covers leading
magic through the byte before DataEnd. Summary CRC covers Statistics through
Footer.summary_offset_start, including the Footer opcode/length. A mathematically
zero CRC still has MCAP's “unavailable” wire interpretation; checksums are not
authentication. The imported historical interleaved-summary exception remains
restricted to its previous profile/library rules and is not used by new output.

## Semantic preservation and migration

The generic API writes full schema bytes/encodings, explicit schema/channel IDs,
topics, message encodings, metadata, opaque payloads, sequences and independent
raw u64 nanosecond publishing/logging times. All integer endpoints are retained.
Definitions and metadata maps use deterministic key order; caller record order is
preserved. Schema ID zero rejects; channel ID zero and schemaless channels work.
No encoding decoder or schema validity claim is inferred from container support.
Clock labels are retained verbatim; missing labels remain unknown. These timestamps
grant no command authority and never supply trusted runtime evaluation time.

`McapWriter::new`, `write_record` and `finish` remain available as a checked legacy
facade; `with_limits`, `flush` and `is_failed` are added. Initialization now emits
the manifest and definitions in manifest order. Default budgets may reject formerly
unbounded recordings; applications must deliberately choose larger validated limits.
Unknown/duplicate manifest channels, unknown clock domains, per-record domain
disagreement, sequence >u32::MAX and timestamps outside 0..=u64::MAX reject.
No default channel/domain is invented. Its single timestamp supplies both times;
use `McapStreamWriter` for full schemas or distinct times. New bytes differ from
the old buffered encoding while valid legacy manifest/sample/digest meaning remains.
The historical reader and bounded importer remain supported.

CLI export is still a checked legacy conversion and rejects generic archives whose
extra metadata, indexes, schema representation or times cannot survive it. Output
is streamed; MCAP input materialization retains its import budgets, while native
input hardening remains separate. The minimal-depth-stream example shows explicit
writer limits; its small caller-owned Vec is not a bounded-memory File example.
`mcap_export` demonstrates the generic File-backed API used for qualification.

## Verification

Implementation `8674da3e21753c8207d2b322ae7645242d87eb5d` passed
[CI 34337776482](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34337776482)
on Ubuntu 24.04 x86_64 with Rust/Cargo 1.94.1 and locked dependencies.
The PR tracks final-revision checks after the evidence/comment update.

Passed on that exact revision:

- `cargo fmt --all --check` and `cargo clippy --locked --workspace --all-targets -- -D warnings`.
- Pinned Python fixture regeneration; 44 recording tests/doctests under the
  120-second outer timeout, including ten externally timed writer cases and
  twelve externally timed import cases.
- Official Python reader agreement on the 586-byte generic export: three messages,
  all schema/channel/encoding/metadata fields, u32/u64 endpoints, empty/binary
  payloads, independent times, record grouping, offsets and both CRCs.
- Declared writer and integrated importer RSS workloads, all within budget.
- 88 focused A04 command regressions; 75 Python-crate tests/doctests under a
  120-second timeout and six SDK tests under a 30-second timeout.
- 336 workspace tests/doctests under a 180-second outer timeout, including
  architecture and CLI destination-preservation/migration regressions.
- Four integrated examples: minimal-depth-stream, auv-depth-sim,
  embedded-propulsion and bounded Python worker with its trusted launcher.
  The additional mcap_export/mcap_import_memory examples built and executed.
- Four independent `--no-default-features` checks: time, command-core,
  embedded-transport and embedded-core; `cargo doc --locked --workspace --no-deps`.
- Two separately invoked AVR compiler/conformance tests. No physical board test
  or rig timing/safe-response validation is claimed.

Counts exclude duplicate child-process reports. Focused suites overlap the
workspace total. Intermediate runs failed formatting and a duplicate-key test's
assumption about the parser's public error wording; these were corrected before
the passing run. No required final compiler check was silently skipped.

The writer test suite uses external 15-second child deadlines and the recording
suite's 120-second CI timeout. Cases cover observable writes, inclusive limits,
invalid configuration/overflow, aliases/conflicts, an injected I/O failure at every
byte, partial/zero writes, flush/finalization failure, abort/Drop, structural CRCs,
legacy rejection and historical summaries. The CLI test verifies an existing
destination survives failed export and no provisional file remains.

Independent reader provenance: official Python `mcap 1.3.1`, pinned in the existing
fixture requirements with `lz4 4.4.4` and `zstandard 0.23.0`. `verify_export.py`
checks exact data records with StreamReader and independently checks footer/CRC
bytes with struct/zlib. The Python implementation is separate from the Rust writer;
this is not solely a Neuradix round-trip. Existing independent import fixture hashes
and generator versions are unchanged. See [fixture tooling](../../crates/record/tests/fixtures/README.md).

Memory workload declared before measurement: Linux File output with fixed one
schema/two channels/one metadata record, 64 KiB borrowed payloads and 8/128 MiB total
payloads (128/2,048 messages). Each child must peak at <=64 MiB RSS; large-versus-small
growth must be <=12 MiB. Producer deadline 20 seconds, wrapper deadline 90 seconds.
Measured child peaks in the referenced CI:

| Workload | Peak RSS | Declared budget |
|---|---:|---:|
| Writer, 8 MiB / 128 messages | 3,024 KiB | 64 MiB |
| Writer, 128 MiB / 2,048 messages | 3,128 KiB | 64 MiB |
| Import streaming, 8 / 128 MiB | 4,208 / 4,132 KiB | 64 MiB each |
| Import materialization, 128 MiB | 135,040 KiB | 192 MiB |
| Import rejecting at 8 MiB retained budget | 11,948 KiB | 64 MiB |

Writer growth was 104 KiB, below 12 MiB. Its accounted state stayed 12,748 bytes;
complete files grew from 8,393,051 to 134,281,691 bytes. Import streaming growth
also stayed within tolerance. This is workload/platform evidence, not universal
production throughput, all allocator behavior or maximum-configuration qualification.

Local Rust/rustup/Cargo and AVR tools are unavailable, so compiler checks run in
pinned GitHub CI (Rust/Cargo 1.94.1, locked dependencies, warnings denied). Local
pinned fixture regeneration, six SDK tests and staged whitespace checks passed.
Changed-document checks passed: nine Markdown files, 332 local links/anchors,
zero errors. The full scan failed on 32 pre-existing archive references across
50 Markdown files/648 links; unrelated repairs are out of scope.
Physical hardware/rig timing and safe-response checks are unavailable and unclaimed.

## Remaining acceptance

This increment covers uncompressed output and a declared independent-reader/memory
workload only. WP-A06/ACC-08 remain partial pending integration and broader
interchange/scale qualification. Compressed writing, seeking/index features,
attachments, live recording, ROS payload adapters and arbitrary replay execution
remain deferred. Container exchange is not decoded ROS/viewer interoperability.
WP-A07 still needs a reusable runner that actually executes the selected program;
CLI replay currently verifies recorded-data integrity.

WP-A05/ACC-09 stay partial for broader resource/platform requirements; WP-A04 stays
partial, ACC-05 incomplete and Gate A open. Conventional local control remains
independent of recording, Python, AI and cloud. No hardware acceptance follows from
host tests or no_std/AVR compilation.
