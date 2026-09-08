# RFC-0021 — MCAP Recording Backend

- Status: Partially implemented (foundation increment 11)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §11 (recording/replay), §41 (tooling interop); [Implementation Plan v0.3](../Neuradix_Implementation_Plan_v0.3.md) Phase 2 (MCAP); complements RFC-0015
- Crate: `neuradix-record`; CLI: `neuradix record export`, `neuradix record inspect`
- External format: [MCAP](https://mcap.dev)

## Problem

The native `.nrec` container is deterministic and self-describing, but it is a
Neuradix-only format. The ecosystem the platform must interoperate with —
Foxglove, ROS 2 tooling, existing visualizers — reads [MCAP](https://mcap.dev).
RFC-0015 always intended MCAP to be "added later behind the same reader/writer
surface". This increment delivers that: an MCAP backend, and — just as important
— the proof that a recording read from *either* container is behaviourally
identical (same replay digest).

## Scope

Implemented in this increment: a backend-neutral [`Recording`] trait; an
uncompressed, spec-compliant MCAP **writer** (`McapWriter`) mirroring
`NativeRecordWriter`'s method set; a matching MCAP **reader** (`McapRecording`);
a generic `replay_digest` defined over the trait; and CLI `record export`
(native → MCAP) plus `record inspect`/`replay run`/`explain` that transparently
accept both containers. Out of scope for this increment: MCAP chunk compression
(LZ4/Zstd), message/chunk indexes, attachments, reading third-party MCAP with a
real schema (schema `data` here is the Neuradix content-addressed identity), and
live streaming writes.

## Proposed decision

### One recording surface, two containers

A recording is a manifest plus ordered [`RawRecord`]s regardless of container.
The [`Recording`] trait (`manifest`, `records`, `count_for`, `records_for`)
captures that, and `NativeRecording` and `McapRecording` both implement it.
Digest, inspection, replay and lineage `explain` are written once against the
trait, so adding MCAP changed no consumer logic. `replay_digest` became generic
(`replay_digest<R: Recording + ?Sized>`), which is why a native `.nrec` and its
MCAP export share a digest.

### The MCAP mapping

The writer emits `MAGIC · Header · Metadata(manifest) · {Schema,Channel}* ·
Message* · DataEnd · {Schema,Channel}* Statistics(summary) · Footer · MAGIC`,
all little-endian per the MCAP record framing (opcode `u8` + `u64` length).
Design choices:

- **Manifest fidelity.** The full Neuradix manifest is embedded losslessly as an
  MCAP `Metadata` record (`neuradix.manifest` → JSON), so a round-trip preserves
  writer id, provenance, seed and note — data MCAP's own structures do not model.
- **Clock domain.** MCAP has no clock-domain concept, so each channel records its
  domain (sourced from its first sample) in `Channel` metadata; the reader
  reconstructs every record's domain from there. This keeps timestamps faithful.
- **Opaque payloads.** Payloads are recorded with `message_encoding = "neuradix"`
  and the payload's content-addressed schema identity is carried as the MCAP
  `Schema` record's data (`encoding = "neuradix/schema-id"`). A first backend does
  not re-encode payloads into a foreign schema language.
- **CRCs.** `data_section_crc32` and `summary_crc32` are written as `0`
  ("not computed"), which the spec permits, avoiding a checksum dependency.

### Safety of parsing

The reader is a bounds-checked, panic-free cursor (mirroring the native reader):
every field read is length-checked, string fields validate UTF-8, and structural
problems (`bad magic`, `unknown channel`, truncation) are typed
`RecordError::Mcap`/`Truncated` values, never panics. Writer casts that could lose
data — a sequence beyond `u32`, a negative or `>u64` timestamp — are rejected with
typed errors rather than silently truncated.

## Public interfaces affected

`neuradix-record`: new `Recording` trait; `McapWriter`, `McapRecording`,
`MCAP_MAGIC`; `replay_digest` is now generic over `Recording`; new
`RecordError::{SequenceTooLarge, TimestampOutOfRange, Mcap}`. CLI: `record export
<file> --out <path>`, and `record inspect`/`replay run`/`explain` accept native
or MCAP (format detected by magic bytes; the result envelope reports `format`).
No internal crate dependencies were added (the backend uses only `neuradix-time`
and the existing model).

## Alternatives considered

- **Depend on the official `mcap` crate.** Rejected for a first backend: it pulls
  compression/index dependencies and a different error surface. A hand-written,
  spec-compliant uncompressed writer/reader matches how the native container was
  built (minimal, bounds-checked, zero heavy deps) and keeps the format under our
  control. The `mcap` crate remains an option if chunking/indexes are needed.
- **Reconstruct the manifest from MCAP channels/statistics only.** Rejected: it is
  lossy (writer id, seed, provenance, note). Embedding the manifest as a metadata
  record makes the round-trip exact while still producing a valid MCAP other tools
  can read.
- **Make MCAP the only container.** Rejected: the native format is simpler, is the
  determinism reference, and has no optional-feature ambiguity. MCAP is the
  interop/export target; native remains the canonical on-disk form.

## Safety and security implications

Replay equivalence across containers is the safety-relevant property: an MCAP
export of a mission replays to the identical digest, so exporting for external
analysis cannot silently alter recorded behaviour (CLI exit code 9 still guards
mismatches). Parsing untrusted MCAP is bounds-checked and panic-free.

## Compatibility implications

`Recording` is additive; existing `NativeRecording`/`NativeRecordWriter` code is
unchanged. `replay_digest`'s generic signature accepts all prior call sites. The
MCAP mapping is versioned by the embedded manifest and the `message_encoding`
string; adding compression or indexes is additive to the container.

## Testing strategy

`crates/record/tests/mcap.rs` covers magic/opcode structure, full record +
manifest round-trip (multi-channel, multi-domain), **native↔MCAP replay-digest
equivalence**, deterministic byte output, the empty recording, bad magic,
truncation, and rejected out-of-range sequence/timestamp. `crates/cli/tests/
record.rs` exercises `record export` → inspect-as-MCAP → replay-against-native-
digest end to end. The writer and reader are independently audited against the
MCAP specification.

## Unresolved questions

- MCAP chunking with LZ4/Zstd compression, plus message/chunk indexes and the
  summary-offset section (faster seeking in large recordings).
- CRC32 computation (currently written as 0 / "not computed").
- Reading third-party MCAP whose payloads use a real schema language
  (Protobuf/JSON Schema/ROS 2) rather than the Neuradix schema identity.
- Recording a simulated mission (RFC-0020) directly to MCAP in one step.
