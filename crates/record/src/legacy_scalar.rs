//! Explicit migration of legacy declaration-order scalar payloads to the
//! canonical `neuradix.scalar-le.v2` encoding (WP-A02).
//!
//! ## The supported historical format
//!
//! Exactly one historical producer is supported: the embedded code generator at
//! revision [`LEGACY_PRODUCER_REVISION`] (`c8aa467`), `nostd-rust` target. Its
//! payloads are the contract's scalar fields in **authored declaration order**,
//! each fixed-width little-endian (IEEE-754 binary64/binary32, two's-complement
//! integers, `bool` as one `0`/`1` byte), with no header and no padding. That
//! codec was never versioned; [`LEGACY_CODEC_LABEL`] is a descriptive label for
//! this pinned format, not an identifier that ever appeared on a wire.
//!
//! The C++ projection at that revision copied eight bytes from a `double`,
//! whose width is target-dependent (four bytes on AVR). Its bytes cannot be
//! interpreted without the target ABI, so it is rejected, as is any other
//! generator or revision.
//!
//! ## What migration never does
//!
//! Field order is not recoverable from schema identity (it is order-independent)
//! or from payload length (equal-width fields permute freely). Migration never
//! guesses. It requires explicit [`LegacyProvenance`] naming the producer
//! revision, the authored contract source and its field order, and it rejects
//! anything missing, inconsistent or unsupported. A native recording with
//! `format_version` 1, or a channel without wire metadata, is **not** assumed to
//! contain legacy scalar payloads: container version, scalar codec version and
//! channel-manifest version are independent.
//!
//! The pinned decoder is stricter than the historical one: a payload must be
//! exactly the legacy length (the historical decoder ignored trailing bytes) and
//! every `bool` byte must be `0` or `1` (the historical decoder accepted any
//! non-zero byte, but its encoder only ever wrote `0` or `1`). Such payloads are
//! rejected rather than normalized.
//!
//! Conversion copies each field's bytes from its legacy offset to its canonical
//! offset, so values (including `-0.0` and NaN payloads) are preserved bit for
//! bit. Channels not named in the provenance are copied unchanged, as opaque
//! bytes. Channel ids, sequences, timestamps, clock domains, record order, the
//! writer, seed, note and software entries are preserved; one software entry is
//! appended to record the migration.

use std::collections::BTreeSet;
use std::path::Path;

use neuradix_contracts::layout::{WireLayout, field_size};
use neuradix_contracts::{Contract, PrimitiveType, schema_identity, validate};
use serde::{Deserialize, Serialize};

use crate::digest::replay_digest;
use crate::model::{ChannelWire, RawRecord, RecordingManifest, SoftwareId};
use crate::native::NativeRecordWriter;
use crate::recording::Recording;

/// The only supported historical producer revision (full commit identity).
pub const LEGACY_PRODUCER_REVISION: &str = "c8aa4671bcee8739354beb7880551db7f64314fa";
/// The only supported historical generator target at that revision.
pub const LEGACY_GENERATOR: &str = "nostd-rust";
/// Descriptive label of the pinned historical format (never on a wire).
pub const LEGACY_CODEC_LABEL: &str =
    "neuradix.legacy.declaration-order-le@c8aa4671bcee8739354beb7880551db7f64314fa";
/// Provenance document format.
pub const PROVENANCE_VERSION: &str = "neuradix.legacy-scalar-provenance.v1";
/// Software entry appended to a migrated manifest.
pub const MIGRATION_SOFTWARE: &str = "neuradix-record/legacy-scalar-migration";

/// Upper bound on a provenance document, in bytes.
pub const MAX_PROVENANCE_BYTES: usize = 1 << 20;
/// Upper bound on one embedded contract source, in bytes.
pub const MAX_CONTRACT_SOURCE_BYTES: usize = 64 << 10;
/// Upper bound on migrated channels per request.
pub const MAX_MIGRATED_CHANNELS: usize = 256;

/// One authored field, as the legacy producer laid it out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyField {
    /// Field name.
    pub name: String,
    /// Contract primitive spelling (for example `float64`).
    #[serde(rename = "type")]
    pub ty: String,
}

/// The producer that wrote the legacy payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyProducer {
    /// Full 40-hex-digit commit of the generator that produced the payloads.
    pub revision: String,
    /// Generator target at that revision.
    pub generator: String,
}

/// Provenance for one legacy channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyChannelProvenance {
    /// Recording channel id.
    pub channel_id: u16,
    /// Schema identity the channel was recorded with.
    pub schema_id: String,
    /// Authored contract source (YAML) the producer generated from.
    pub contract: String,
    /// Authored field order, which is the legacy wire order.
    pub field_order: Vec<LegacyField>,
}

/// An explicit legacy migration request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyProvenance {
    /// Must equal [`PROVENANCE_VERSION`].
    pub provenance_version: String,
    /// The historical producer.
    pub producer: LegacyProducer,
    /// Channels whose payloads are legacy scalar payloads.
    pub channels: Vec<LegacyChannelProvenance>,
}

impl LegacyProvenance {
    /// Parse a bounded provenance document (unknown fields are rejected).
    pub fn from_json(bytes: &[u8]) -> Result<Self, MigrationError> {
        if bytes.len() > MAX_PROVENANCE_BYTES {
            return Err(MigrationError::ProvenanceTooLarge(bytes.len()));
        }
        serde_json::from_slice(bytes).map_err(|e| MigrationError::Provenance(e.to_string()))
    }
}

/// A decoded legacy scalar value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LegacyScalar {
    /// binary64.
    F64(f64),
    /// binary32.
    F32(f32),
    /// Signed 32-bit.
    I32(i32),
    /// Signed 64-bit.
    I64(i64),
    /// Unsigned 32-bit.
    U32(u32),
    /// Unsigned 64-bit.
    U64(u64),
    /// Boolean (`0` or `1` on the wire).
    Bool(bool),
}

#[derive(Debug, Clone)]
struct Slot {
    name: String,
    ty: PrimitiveType,
    offset: usize,
    size: usize,
}

/// The pinned legacy layout of one contract, verified against its provenance.
#[derive(Debug, Clone)]
pub struct LegacyLayout {
    slots: Vec<Slot>,
    wire_len: usize,
    canonical: WireLayout,
}

impl LegacyLayout {
    /// Build and cross-check the legacy layout for one provenance channel:
    /// size bounds, contract validation, contract schema identity against the
    /// provenance, authored field order against the declared order, and
    /// fixed-width scalar fields only. It does not consult any recording.
    pub fn new(entry: &LegacyChannelProvenance) -> Result<Self, MigrationError> {
        let channel = entry.channel_id;
        if entry.contract.len() > MAX_CONTRACT_SOURCE_BYTES {
            return Err(MigrationError::ContractTooLarge {
                channel,
                bytes: entry.contract.len(),
            });
        }
        let contract = validate::from_yaml_str(&entry.contract, Path::new("<provenance>"))
            .map_err(|e| MigrationError::Contract {
                channel,
                reason: e.to_string(),
            })?;
        if schema_identity(&contract).as_str() != entry.schema_id {
            return Err(MigrationError::SchemaMismatch {
                channel,
                reason: "contract source does not produce the provenance schema identity",
            });
        }
        Self::from_contract(channel, &contract, &entry.field_order)
    }

    fn from_contract(
        channel: u16,
        contract: &Contract,
        field_order: &[LegacyField],
    ) -> Result<Self, MigrationError> {
        let authored = &contract.spec.payload.fields;
        let declared = authored.len() == field_order.len()
            && authored
                .iter()
                .zip(field_order)
                .all(|(a, d)| a.name == d.name && a.ty.as_contract_str() == d.ty);
        if !declared {
            return Err(MigrationError::FieldOrderMismatch { channel });
        }
        let mut slots = Vec::with_capacity(authored.len());
        let mut offset = 0usize;
        for field in authored {
            let size = field_size(field.ty, &field.name).map_err(|e| {
                MigrationError::UnsupportedField {
                    channel,
                    reason: e.to_string(),
                }
            })?;
            slots.push(Slot {
                name: field.name.clone(),
                ty: field.ty,
                offset,
                size,
            });
            offset = offset
                .checked_add(size)
                .ok_or(MigrationError::UnsupportedField {
                    channel,
                    reason: "layout size overflow".into(),
                })?;
        }
        let canonical =
            WireLayout::for_contract(contract).map_err(|e| MigrationError::UnsupportedField {
                channel,
                reason: e.to_string(),
            })?;
        debug_assert_eq!(canonical.wire_len, offset);
        Ok(Self {
            slots,
            wire_len: offset,
            canonical,
        })
    }

    /// Exact legacy payload length.
    pub fn wire_len(&self) -> usize {
        self.wire_len
    }

    /// The canonical v2 layout the payloads migrate to.
    pub fn canonical(&self) -> &WireLayout {
        &self.canonical
    }

    fn check(&self, payload: &[u8]) -> Result<(), PayloadError> {
        if payload.len() != self.wire_len {
            return Err(PayloadError::Length {
                expected: self.wire_len,
                actual: payload.len(),
            });
        }
        for slot in self.slots.iter().filter(|s| s.ty == PrimitiveType::Bool) {
            let byte = payload[slot.offset];
            if byte > 1 {
                return Err(PayloadError::NonCanonicalBool {
                    field: slot.name.clone(),
                    byte,
                });
            }
        }
        Ok(())
    }

    /// Decode one legacy payload into `(field, value)` pairs in declaration
    /// order, with the strict rules in the module documentation.
    pub fn decode(&self, payload: &[u8]) -> Result<Vec<(String, LegacyScalar)>, PayloadError> {
        self.check(payload)?;
        Ok(self
            .slots
            .iter()
            .map(|s| {
                let b = &payload[s.offset..s.offset + s.size];
                let value = match s.ty {
                    PrimitiveType::Float64 => LegacyScalar::F64(f64::from_le_bytes(arr(b))),
                    PrimitiveType::Float32 => LegacyScalar::F32(f32::from_le_bytes(arr(b))),
                    PrimitiveType::Int32 => LegacyScalar::I32(i32::from_le_bytes(arr(b))),
                    PrimitiveType::Int64 => LegacyScalar::I64(i64::from_le_bytes(arr(b))),
                    PrimitiveType::Uint32 => LegacyScalar::U32(u32::from_le_bytes(arr(b))),
                    PrimitiveType::Uint64 => LegacyScalar::U64(u64::from_le_bytes(arr(b))),
                    PrimitiveType::Bool => LegacyScalar::Bool(b[0] == 1),
                    PrimitiveType::Str => unreachable!("rejected by field_size"),
                };
                (s.name.clone(), value)
            })
            .collect())
    }

    /// Convert one legacy payload to canonical v2 bytes (field bytes moved from
    /// their legacy offsets to their canonical offsets).
    pub fn to_canonical(&self, payload: &[u8]) -> Result<Vec<u8>, PayloadError> {
        self.check(payload)?;
        let mut out = vec![0u8; self.canonical.wire_len];
        for field in &self.canonical.fields {
            let slot = self
                .slots
                .iter()
                .find(|s| s.name == field.name)
                .expect("canonical and legacy layouts share fields");
            debug_assert_eq!(slot.size, field.size);
            out[field.offset..field.offset + field.size]
                .copy_from_slice(&payload[slot.offset..slot.offset + slot.size]);
        }
        Ok(out)
    }
}

fn arr<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes.try_into().expect("slot width matches type")
}

/// Why one payload could not be migrated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PayloadError {
    /// Not exactly the legacy length.
    #[error("payload is {actual} bytes, legacy layout is {expected}")]
    Length {
        /// Legacy length.
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// A `bool` byte other than `0` or `1`.
    #[error("field `{field}` has non-canonical bool byte {byte:#04x}")]
    NonCanonicalBool {
        /// Field name.
        field: String,
        /// Offending byte.
        byte: u8,
    },
}

/// Why a migration request was rejected. Nothing is produced on error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    /// The provenance document exceeds [`MAX_PROVENANCE_BYTES`].
    #[error("provenance document too large ({0} bytes)")]
    ProvenanceTooLarge(usize),
    /// Malformed provenance JSON or unknown fields.
    #[error("invalid provenance: {0}")]
    Provenance(String),
    /// Unknown provenance format.
    #[error("unsupported provenance version `{0}`")]
    UnsupportedProvenanceVersion(String),
    /// Not the pinned producer revision (abbreviations are not accepted).
    #[error("unsupported legacy producer revision `{0}`")]
    UnsupportedRevision(String),
    /// Not the pinned generator target.
    #[error("unsupported legacy generator `{0}`")]
    UnsupportedGenerator(String),
    /// No channels named: there is nothing the provenance establishes.
    #[error("provenance names no channels")]
    NoChannels,
    /// More channels than [`MAX_MIGRATED_CHANNELS`].
    #[error("provenance names too many channels ({0})")]
    TooManyChannels(usize),
    /// A channel is named twice.
    #[error("channel {0} appears twice in the provenance")]
    DuplicateChannel(u16),
    /// The recording has no such channel.
    #[error("recording has no channel {0}")]
    UnknownChannel(u16),
    /// The channel already records a wire binding; it is not a legacy stream.
    #[error("channel {0} already records wire metadata")]
    AlreadyBound(u16),
    /// The embedded contract source exceeds [`MAX_CONTRACT_SOURCE_BYTES`].
    #[error("channel {channel}: contract source too large ({bytes} bytes)")]
    ContractTooLarge {
        /// Channel.
        channel: u16,
        /// Size.
        bytes: usize,
    },
    /// The embedded contract does not parse or validate.
    #[error("channel {channel}: invalid contract source: {reason}")]
    Contract {
        /// Channel.
        channel: u16,
        /// Validation error.
        reason: String,
    },
    /// Contract, provenance and recording schema identities disagree.
    #[error("channel {channel}: {reason}")]
    SchemaMismatch {
        /// Channel.
        channel: u16,
        /// Which pair disagreed.
        reason: &'static str,
    },
    /// The declared field order is not the contract's authored order.
    #[error("channel {channel}: field order does not match the authored contract")]
    FieldOrderMismatch {
        /// Channel.
        channel: u16,
    },
    /// A field has no fixed scalar width (for example `string`).
    #[error("channel {channel}: {reason}")]
    UnsupportedField {
        /// Channel.
        channel: u16,
        /// Reason.
        reason: String,
    },
    /// A record's payload is malformed for the legacy layout.
    #[error("channel {channel} sequence {sequence}: {error}")]
    Payload {
        /// Channel.
        channel: u16,
        /// Record sequence.
        sequence: u64,
        /// Record index in recording order.
        index: usize,
        /// Payload error.
        error: PayloadError,
    },
    /// Writing the migrated container failed.
    #[error("cannot encode migrated recording: {0}")]
    Encode(String),
}

/// Per-channel migration result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigratedChannel {
    /// Channel id.
    pub channel_id: u16,
    /// Records converted.
    pub records: usize,
    /// Legacy payload length.
    pub legacy_wire_len: usize,
    /// Resulting codec.
    pub codec_id: String,
    /// Resulting full wire identity.
    pub wire_id: String,
}

/// What a migration did, for audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    /// Source format label ([`LEGACY_CODEC_LABEL`]).
    pub source_codec: String,
    /// Producer revision taken from the provenance.
    pub source_revision: String,
    /// Replay digest of the source records.
    pub source_digest: String,
    /// Replay digest of the migrated records (differs when bytes moved).
    pub migrated_digest: String,
    /// Converted channels, in provenance order.
    pub channels: Vec<MigratedChannel>,
}

/// A migrated recording: manifest, records and report.
#[derive(Debug, Clone)]
pub struct MigratedRecording {
    manifest: RecordingManifest,
    records: Vec<RawRecord>,
    report: MigrationReport,
}

impl MigratedRecording {
    /// The audit report.
    pub fn report(&self) -> &MigrationReport {
        &self.report
    }

    /// Serialize as a native recording (container `format_version` 1).
    pub fn to_native_bytes(&self) -> Result<Vec<u8>, MigrationError> {
        let encode = |e: crate::RecordError| MigrationError::Encode(e.to_string());
        let mut writer = NativeRecordWriter::new(Vec::new(), &self.manifest).map_err(encode)?;
        for r in &self.records {
            writer
                .write_record(r.channel_id, r.sequence, r.timestamp, &r.payload)
                .map_err(encode)?;
        }
        writer.finish().map_err(encode)
    }
}

impl Recording for MigratedRecording {
    fn manifest(&self) -> &RecordingManifest {
        &self.manifest
    }

    fn records(&self) -> &[RawRecord] {
        &self.records
    }
}

/// Migrate the channels named by `provenance` from the pinned legacy scalar
/// format to `neuradix.scalar-le.v2`. All checks run before any output is
/// produced; any failure rejects the whole request.
pub fn migrate_legacy_scalar<R: Recording + ?Sized>(
    recording: &R,
    provenance: &LegacyProvenance,
) -> Result<MigratedRecording, MigrationError> {
    if provenance.provenance_version != PROVENANCE_VERSION {
        return Err(MigrationError::UnsupportedProvenanceVersion(
            provenance.provenance_version.clone(),
        ));
    }
    if provenance.producer.revision != LEGACY_PRODUCER_REVISION {
        return Err(MigrationError::UnsupportedRevision(
            provenance.producer.revision.clone(),
        ));
    }
    if provenance.producer.generator != LEGACY_GENERATOR {
        return Err(MigrationError::UnsupportedGenerator(
            provenance.producer.generator.clone(),
        ));
    }
    if provenance.channels.is_empty() {
        return Err(MigrationError::NoChannels);
    }
    if provenance.channels.len() > MAX_MIGRATED_CHANNELS {
        return Err(MigrationError::TooManyChannels(provenance.channels.len()));
    }
    let source = recording.manifest();
    let mut seen = BTreeSet::new();
    let mut layouts = Vec::with_capacity(provenance.channels.len());
    for entry in &provenance.channels {
        let id = entry.channel_id;
        if !seen.insert(id) {
            return Err(MigrationError::DuplicateChannel(id));
        }
        let channel = source
            .channel(id)
            .ok_or(MigrationError::UnknownChannel(id))?;
        if channel.wire.is_some() {
            return Err(MigrationError::AlreadyBound(id));
        }
        if channel.schema_id != entry.schema_id {
            return Err(MigrationError::SchemaMismatch {
                channel: id,
                reason: "recording channel schema identity differs from the provenance",
            });
        }
        layouts.push((id, LegacyLayout::new(entry)?));
    }
    let layout_for = |id: u16| layouts.iter().find(|(c, _)| *c == id).map(|(_, l)| l);

    let mut records = Vec::with_capacity(recording.records().len());
    let mut counts = vec![0usize; layouts.len()];
    for (index, record) in recording.records().iter().enumerate() {
        let payload = match layout_for(record.channel_id) {
            Some(layout) => {
                let slot = layouts
                    .iter()
                    .position(|(c, _)| *c == record.channel_id)
                    .expect("present");
                counts[slot] += 1;
                layout
                    .to_canonical(&record.payload)
                    .map_err(|error| MigrationError::Payload {
                        channel: record.channel_id,
                        sequence: record.sequence,
                        index,
                        error,
                    })?
            }
            None => record.payload.clone(),
        };
        records.push(RawRecord {
            channel_id: record.channel_id,
            sequence: record.sequence,
            timestamp: record.timestamp,
            payload,
        });
    }

    let mut manifest = source.clone();
    for channel in &mut manifest.channels {
        if let Some(layout) = layout_for(channel.id) {
            channel.wire = Some(ChannelWire::for_layout(layout.canonical()));
        }
    }
    manifest.software.push(SoftwareId::new(
        MIGRATION_SOFTWARE,
        format!(
            "{} from {}",
            env!("CARGO_PKG_VERSION"),
            provenance.producer.revision
        ),
    ));
    let channels = layouts
        .iter()
        .zip(&counts)
        .map(|((id, layout), &n)| MigratedChannel {
            channel_id: *id,
            records: n,
            legacy_wire_len: layout.wire_len(),
            codec_id: layout.canonical().codec_id.clone(),
            wire_id: layout.canonical().wire_id.clone(),
        })
        .collect();
    let mut migrated = MigratedRecording {
        manifest,
        records,
        report: MigrationReport {
            source_codec: LEGACY_CODEC_LABEL.into(),
            source_revision: provenance.producer.revision.clone(),
            source_digest: replay_digest(recording),
            migrated_digest: String::new(),
            channels,
        },
    };
    migrated.report.migrated_digest = replay_digest(&migrated);
    Ok(migrated)
}
