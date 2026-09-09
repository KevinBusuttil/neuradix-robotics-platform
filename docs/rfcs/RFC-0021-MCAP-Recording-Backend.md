# RFC-0021 — MCAP Recording Backend

- Status: Partially implemented; bounded import increment under WP-A06
- Authoritative plan: [Implementation Plan v0.4](../Neuradix_Implementation_Plan_v0.4.md)
- Crate: `neuradix-record`; CLI: `record inspect`, `record export`
- Related: [RFC-0015](RFC-0015-Recording-and-Deterministic-Replay.md)
- Evidence: [WP-A06 bounded MCAP import](../implementation/WP-A06-Bounded-MCAP-Import.md)

## Problem and decision

The original uncompressed MCAP reader silently ignored unsupported records,
including message-bearing chunks. It discarded full schemas, encodings and
publishing times; whole-file loading had no resource policy. A matching local
writer/reader round-trip did not demonstrate external interchange.

Replace import with pinned maintained `mcap 0.25.0` parsing, wrapped in validated
resource limits and allocation-free field preflight. Emit chunks from the parser
without automatic expansion. Support uncompressed chunks and capped LZ4 decoding
through `lz4 1.28.1`; reject other compression and unsupported records explicitly.
Validate available CRCs, section structure, summary offsets and definition
references. Index bodies are retained opaquely and never used to skip data.

## Semantic surface and bounded ownership

`McapImportLimits` is private and validated. It bounds file bytes, record/message
sizes and counts, chunk expansion (per chunk and total), schema/channel counts,
map entries, persistent definitions and retained archive storage. Exact defaults,
ceilings, decoder scratch and measured RSS limitations are in the evidence.

`import_mcap(Read, limits, visitor)` synchronously yields borrowed events. The
visitor provides backpressure; no queue or background task exists. Caller-owned
copies are outside the streaming budget. Events are provisional until successful
EOF and CRC validation. Callers must not commit irreversible effects from a
prefix. Arbitrary blocking readers/callbacks require external supervision.

`McapArchive` offers bounded materialization and preserves header declarations,
full schemas, opaque schema/message encodings, payloads, metadata, sequences and
both raw `u64` nanosecond times. Their epochs and clock relationship remain
producer-defined; no missing clock label is invented. Supported encodings are
retained, not decoded. Unknown/private opcodes, attachments, additional record
fields and unsupported chunks produce explicit errors, never silent omission.
Identical definitions normalize; conflicting IDs/named metadata reject.

## Compatibility and writer boundary

The historical `Recording` trait represents a Neuradix manifest and one timestamp
per message. `McapArchive::try_into_recording` checks whether this projection is
faithful. It requires the historical Neuradix profile, matching schemas and clock
metadata, equal source/log times, supported manifest fields and no extra metadata
or indexes that would be lost. Generic external archives use the new surface.

`McapRecording::from_bytes` retains its signature but applies default bounds and
strict conversion. `from_reader` permits explicit validated limits. Existing
Neuradix recordings within those limits preserve their manifests and replay
digests. MCAP inspect uses bounded File input and exposes generic metadata;
unsupported legacy digest conversion is reported as null. Replay/explain/export
reject unrepresentable conversion. Native loading is unchanged.

`McapWriter` still buffers all records until finish and writes uncompressed
Neuradix encodings with a manifest metadata record. Its zero CRC fields mean
checksums are absent. Writer redesign, live streaming and bounded write behavior
are deferred. Per-channel domains originate in manifest/sample validation; import
requires manifest/channel agreement instead of assuming a domain from log time.
Historical 0.0.1 summary repetitions interleave schemas/channels. Import admits
this documented compatibility exception only for that declared profile/library,
with matching definitions and no summary-offset section. It does not certify the
old writer as strictly conforming to MCAP summary grouping.

## Alternatives and evidence

The earlier decision to keep a handwritten reader is superseded by WP-A06.
Unrestricted upstream automatic decompression also fails the required allocation
policy; explicit preflight and capped decoding remain necessary. Silently mapping
foreign metadata/times to the old Recording interface is rejected as lossy.

Pinned official Python-generated fixtures establish the import side of independent
container exchange: uncompressed records/chunks and LZ4, with complete schemas,
metadata and separate source/log times. Timed adversarial tests and Linux peak-RSS
workloads check declared bounds. Local round-trips remain compatibility evidence.
The evidence document records exact verification and unsupported platforms.

## Remaining acceptance and safety

Bounded streaming writing, independently decoded supported exports, wider scale
qualification and general replay execution remain separate work. MCAP container
support does not establish decoded ROS/viewer interoperability. Recording import
runs outside conventional local control and grants no command authority. A04/A05
regressions and independent no_std/AVR checks remain required. WP-A06/ACC-08 are
partial and Gate A is open.
