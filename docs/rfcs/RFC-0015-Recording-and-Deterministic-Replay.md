# RFC-0015 — Recording and Deterministic Replay

- Status: Accepted (partially implemented — foundation increment 2)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §24, §7.6; complements RFC-0003
- Crate: `neuradix-record`

## Problem

Deterministic replay is the platform's headline differentiator. To reproduce a
run, a recording must capture not just samples but enough context (channels,
schema identities, clock domains, provenance) to interpret them, and replay must
be verifiable — two replays of the same recording must be provably identical.

## Scope

Implemented in this increment: a native, self-describing recording container; a
payload-agnostic codec; a deterministic replay digest; and `neuradix record
inspect` / `neuradix replay run` (with `--expect-digest` wiring exit code 9).
Out of scope: MCAP, live `record start/stop` against a running graph, branch and
counterfactual replay, and a recording-driven replay clock feeding a live graph.

## Proposed decision

### Native container format (version 1)

```text
magic    : "NRXREC"  (6 bytes)
version  : u8        (= 1)
manifest : u32 length (LE) + UTF-8 JSON manifest
records  : repeated { u16 channel_id, u64 sequence, u8 clock_domain_code,
                      i128 nanos (LE), u32 payload_len (LE), payload bytes }
```

Fixed little-endian, length-prefixed, so recordings round-trip byte-for-byte.
Clock-domain codes are the stable `ClockDomain::code()` values from RFC-0003.
Parsing is fully bounds-checked and never panics; truncated or corrupt input is
a typed `RecordError::Truncated`/`BadMagic`/`UnsupportedVersion`.

### Manifest

Captures `formatVersion`, `writer`, `channels` (id, name, **schema identity**,
clock domain), `software` provenance, optional `seed` and `note`. Schema
identities tie every channel back to its content-addressed contract (RFC-0002).

### Codec

`RecordCodec { encode; decode }` keeps serialization out of the recorder, so the
format does not prematurely commit to a wire encoding. Codecs must be
deterministic (`decode(encode(m)) == m`, identical bytes each time).

### Replay digest

`replay_digest(&recording) -> "sha256:<hex>"` hashes, in order, every record's
channel id, sequence, domain, timestamp and payload (manifest excluded, so
provenance notes don't perturb behaviour). Equal recordings ⇒ equal digest;
any behavioural change ⇒ different digest. `neuradix replay run --expect-digest`
returns exit code **9** (determinism/replay mismatch) when the digest differs.

### Backend neutrality

`NativeRecordWriter`/`NativeRecording` are the first backend. MCAP is intended
as an additional backend behind the same writer/reader surface, mirroring how
`neuradix-transport-api` hides its backend — no recorded component code changes.

## Public interfaces affected

`neuradix-record`: `RecordingManifest` (+ `ManifestBuilder`), `Channel`,
`SoftwareId`, `RawRecord`, `RecordCodec`, `NativeRecordWriter`,
`NativeRecording`, `replay_digest`, `RecordError`. CLI: `record inspect`,
`replay run`. `neuradix-time`: `ClockDomain::code`/`from_code`.

## Alternatives considered

- **Adopt MCAP now.** Deferred: it adds an external dependency and container
  complexity before the neutral surface and determinism story are proven. MCAP
  becomes a backend behind the same interface.
- **Record typed values directly (bake in serialization).** Rejected: keeping
  payloads opaque via a codec avoids committing to a wire format prematurely.
- **Include the manifest in the replay digest.** Rejected: provenance/notes are
  non-behavioural and should not change a behavioural equivalence check.

## Safety and security implications

Recordings capture schema identities and software provenance, supporting the
evidence chain (§3.11) and turning a field incident into a replayable artifact.
The parser is panic-free and bounds-checked against malformed/hostile input.
Recording preserves — never relaxes — clock-domain semantics.

## Compatibility implications

The container is versioned (`magic` + `version`); a new layout bumps the version
and readers reject unknown versions cleanly. Clock-domain codes are part of the
wire format and are fixed. Manifest fields may be added additively (serde
tolerates unknown/missing optional fields).

## Testing strategy

`crates/record/tests/roundtrip.rs`: byte-for-byte round trip (payloads,
timestamps, domains, manifest), digest determinism and sensitivity, and rejection
of bad-magic/truncated input. `crates/cli/tests/record.rs`: `record inspect`,
`replay run` success, and the exit-code-9 mismatch path. The `minimal-depth-stream`
example records its run and verifies replay fidelity at runtime.

## Unresolved questions

- MCAP backend mapping (channels/schemas/attachments) and when to add it.
- Live `record start/stop` against a running graph and a recording-driven replay
  clock that re-drives components in lockstep.
- Branch and counterfactual replay (§24.4) and partial-graph replay.
