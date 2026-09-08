# RFC-0022 — Studio Inspection Model

- Status: Partially implemented (foundation increment 12)
- Authoritative spec: [Functional Specification v0.5](../Neuradix_Robotics_Platform_Functional_Specification_v0.5.md) §20 (Studio), §11 (recording/replay); [Implementation Plan v0.3](../Neuradix_Implementation_Plan_v0.3.md) Phase 2 (Studio inspection); complements RFC-0015, RFC-0021
- Crate: `neuradix-studio`; CLI: `neuradix studio timeline`, `neuradix studio series`

## Problem

Studio (and, later, Studio XR) must let an operator *inspect* what a robot did:
scrub a timeline, see which channels carried what at what rate, and plot a signal
over time. That capability needs a **read model** — a query layer over a
recording — long before any UI exists. Building the queries into the UI would
bind them to a rendering stack and make them untestable; building them into the
recorder would bloat a format that should stay minimal. The inspection model is
therefore its own headless, deterministic layer.

## Scope

Implemented in this increment: an [`Inspection`] index over any
[`neuradix_record::Recording`] (native `.nrec` or MCAP) exposing a timeline
(per-domain spans + per-channel statistics), windowed record access, a
nearest-record query, and scalar time-series extraction via a caller-supplied
decoder; plus `neuradix studio timeline` and `neuradix studio series` (the
latter reading the command-lineage channel). Out of scope for this increment:
live inspection of a running graph, spatial/3-D scene state (RFC-0008), a
transport for streaming updates to a UI, cross-channel joins/aggregation, and
downsampling/decimation for very large series.

## Proposed decision

### A headless read model over the `Recording` trait

Inspection is built over the backend-neutral `Recording` surface from RFC-0021,
so it works identically over a native or MCAP recording and never assumes a
container. It is pure: every query is a deterministic function of the recording,
with no I/O, clock or rendering. This is what lets the same answers back a CLI
table, a Studio panel or an XR scene.

### Timeline and per-channel statistics

`timeline()` returns per-**domain** spans (`DomainSpan`) and per-channel
summaries (`ChannelSummary`: count, first/last time, span, mean period,
effective rate, payload sizes). Domains are reported separately because
cross-domain time arithmetic is not meaningful — a mission mixes, e.g.,
`simulation` control samples and `monotonic` command lineage, and collapsing
them into one range would be a category error. Channels with no records still
appear (from the manifest), so the inventory is complete.

### Windowed and nearest queries

`window(channel, start, end)` returns the records in an inclusive time range;
`nearest(channel, at)` returns the record closest to an instant (ties to the
earlier). Both use binary search over per-channel, time-sorted position indices
built once at construction, so they are efficient on large recordings and never
panic — an inverted range yields an empty window, and timestamp arithmetic is
overflow-safe even at the extremes of the `i128` domain.

### Scalar series through a decoder seam

Payloads are opaque bytes, so turning them into plottable numbers requires a
`ScalarDecoder` the caller supplies. `series(channel, field, decoder)` decodes
each payload, collects the named field with its timestamp in time order, and
computes `SeriesStats` (min/max/mean/first/last). Keeping the decoder external
means the inspection layer commits to no wire encoding: a fixed-layout codec, the
command-lineage reader, or a future Protobuf decoder all plug into the same
query. The CLI ships a lineage decoder, so `studio series --field applied` plots
the safety-gated applied command over a mission.

## Public interfaces affected

`neuradix-studio`: `Inspection`; `Timeline`/`DomainSpan`/`ChannelSummary`;
`ScalarDecoder`/`ScalarSample`/`Series`/`SeriesPoint`/`SeriesStats`;
`StudioError`. CLI: `studio timeline <file>` and `studio series <file>
--field <name> [--channel <id>]`, both accepting native or MCAP. The crate
depends only on `neuradix-record` and `neuradix-time`.

## Alternatives considered

- **Decode payloads natively in Studio.** Rejected: there is no canonical wire
  encoding yet (RFC-0002 leaves it open), so a built-in decoder would either
  guess or hard-code the example's layout. The `ScalarDecoder` seam defers the
  choice to the caller and keeps the crate neutral.
- **Collapse all channels into one global timeline range.** Rejected: it violates
  the platform's clock-domain discipline (§14.1). Per-domain spans are correct
  and still let a UI present a combined ruler if it chooses.
- **Fold inspection into the CLI.** Rejected: the queries must be reusable by the
  eventual Studio/XR front-ends and unit-testable without a UI, which a library
  provides and a CLI subcommand does not.

## Safety and security implications

Inspection is read-only and pure, so it cannot affect a running system. Parsing
still flows through the bounds-checked record readers; the query layer adds no
new parsing. Overflow-safe timestamp handling means a crafted recording cannot
crash an inspector.

## Compatibility implications

`Inspection` queries are additive; new `ChannelSummary`/`SeriesStats` fields are
additive and read by name. The `ScalarDecoder` trait is the stable extension
point for new payload encodings. Nothing here changes the recording formats.

## Testing strategy

`crates/studio/tests/inspection.rs` covers timeline statistics and rate,
multi-domain spans, inclusive windows, nearest tie-breaking and clamping, series
extraction with stats, typed errors, single-record and empty channels,
determinism, and the two adversarial edge cases surfaced by review — an inverted
`window` range and extreme (`i128::MIN`/`MAX`) timestamps — neither of which
panics. `crates/cli/tests/studio.rs` exercises `studio timeline` (channels +
domains) and `studio series` (applied value from auto-selected lineage channel)
end to end.

## Unresolved questions

- Live inspection of a running graph (a streaming query/subscription surface).
- Downsampling/decimation and windowed aggregation for very large series.
- Cross-channel alignment/joins (e.g. requested-vs-applied on one axis).
- The spatial/scene read model Studio XR needs (RFC-0008), built on this base.
