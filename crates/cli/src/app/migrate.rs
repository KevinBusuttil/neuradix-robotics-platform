//! `neuradix record migrate`: explicit, bounded, native-to-native migration of
//! legacy (c8aa467) declaration-order scalar payloads to `neuradix.scalar-le.v2`.
//!
//! All decoding, provenance validation and conversion is
//! `neuradix_record::legacy_scalar`; this module only bounds input, publishes
//! output safely and reports.
//!
//! ## Resource bounds
//!
//! - Recording: at most `--max-input-bytes` container bytes (default and
//!   ceiling 256 MiB) and `--max-records` records (default and ceiling 1 Mi),
//!   enforced while reading ([`NativeRecording::from_reader`]).
//! - Provenance: at most [`MAX_PROVENANCE_BYTES`] (1 MiB), enforced while
//!   reading; each embedded contract at most 64 KiB (library check).
//!
//! Peak memory is roughly two decoded copies of the recording: the read buffer
//! (≤ max + 1 bytes) is released after parsing, the source recording is dropped
//! after conversion, and the output is streamed to disk. Each decoded record
//! also costs a fixed-size [`neuradix_record::RawRecord`] plus its payload allocation, and the
//! record vectors may hold up to twice their length in spare capacity.
//!
//! ## Publication
//!
//! The source is never modified. The destination must not exist, and an
//! existing path that is the input (including a hard link to it) is refused as
//! in-place migration. Output is written to a new temporary file in the
//! destination directory, flushed and `fsync`ed, then published with
//! `link(2)`, which fails rather than replaces a destination created
//! concurrently. Temporary cleanup is best effort (see [`crate::app::publish`]):
//! a leftover temporary file is reported as a warning. Durability of the
//! directory entry across power loss is not claimed.

use std::fs::File;
use std::io::{BufWriter, Read};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use neuradix_record::legacy_scalar::MAX_PROVENANCE_BYTES;
use neuradix_record::{
    LegacyProvenance, MAGIC, NativeReadLimits, NativeRecordWriter, NativeRecording, Recording,
    migrate_legacy_scalar,
};
use serde_json::{Value, json};

use crate::app::publish::{leftover_warning, parent, publish_new};
use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

fn fail(exit: ExitCode, message: impl Into<String>) -> AppError {
    AppError::message(exit, message)
}

/// `neuradix record migrate <file> --provenance <json> --out <file>`.
pub fn migrate(
    file: &Path,
    provenance: &Path,
    out: &Path,
    limits: NativeReadLimits,
) -> Result<Outcome, AppError> {
    let input = std::fs::metadata(file).map_err(|e| {
        fail(
            ExitCode::GeneralFailure,
            format!("could not read recording `{}`: {e}", file.display()),
        )
    })?;
    check_destination(out, &input)?;

    let recording = read_recording(file, limits)?;
    let request = read_provenance(provenance)?;
    let migrated = migrate_legacy_scalar(&recording, &request)
        .map_err(|e| fail(ExitCode::Compatibility, format!("migration refused: {e}")))?;
    let opaque: Vec<u16> = recording
        .manifest()
        .channels
        .iter()
        .map(|c| c.id)
        .filter(|id| {
            !migrated
                .report()
                .channels
                .iter()
                .any(|m| m.channel_id == *id)
        })
        .collect();
    drop(recording);

    let published = publish_new(out, "migrate", |sink| {
        let mut writer = NativeRecordWriter::new(BufWriter::new(sink), migrated.manifest())
            .map_err(std::io::Error::other)?;
        for r in migrated.records() {
            writer
                .write_record(r.channel_id, r.sequence, r.timestamp, &r.payload)
                .map_err(std::io::Error::other)?;
        }
        writer
            .finish()
            .map_err(std::io::Error::other)?
            .into_inner()
            .map_err(|e| e.into_error())?;
        Ok(())
    })
    .map_err(|e| {
        fail(
            ExitCode::GeneralFailure,
            format!("could not publish `{}`: {e}", out.display()),
        )
    })?;

    let report = migrated.report();
    let channels: Vec<Value> = report
        .channels
        .iter()
        .map(|c| {
            json!({
                "channelId": c.channel_id,
                "records": c.records,
                "legacyWireLen": c.legacy_wire_len,
                "codecId": c.codec_id,
                "wireId": c.wire_id,
            })
        })
        .collect();
    let warnings = leftover_warning(&published);
    Ok(Outcome::with_warnings(
        json!({
            "source": file.display().to_string(),
            "file": out.display().to_string(),
            "format": "native",
            "formatVersion": migrated.manifest().format_version,
            "bytes": published.bytes,
            "records": migrated.records().len(),
            "sourceCodec": report.source_codec,
            "sourceRevision": report.source_revision,
            "sourceDigest": report.source_digest,
            "migratedDigest": report.migrated_digest,
            "channels": channels,
            "opaqueChannels": opaque,
            "limits": {
                "maxInputBytes": limits.max_bytes(),
                "maxRecords": limits.max_records(),
                "maxProvenanceBytes": MAX_PROVENANCE_BYTES,
            },
        }),
        warnings,
    ))
}

/// Refuse in-place migration and any existing destination (including dangling
/// symlinks), before reading anything large.
fn check_destination(out: &Path, input: &std::fs::Metadata) -> Result<(), AppError> {
    if out.file_name().is_none() {
        return Err(fail(
            ExitCode::InvalidUse,
            format!("destination `{}` has no file name", out.display()),
        ));
    }
    match std::fs::symlink_metadata(out) {
        Ok(_) => {
            let same = std::fs::metadata(out)
                .is_ok_and(|m| m.dev() == input.dev() && m.ino() == input.ino());
            Err(fail(
                ExitCode::InvalidUse,
                if same {
                    format!(
                        "refusing in-place migration: `{}` is the input recording",
                        out.display()
                    )
                } else {
                    format!("refusing to overwrite existing `{}`", out.display())
                },
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let dir = parent(out);
            if std::fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
                Ok(())
            } else {
                Err(fail(
                    ExitCode::GeneralFailure,
                    format!("destination directory `{}` does not exist", dir.display()),
                ))
            }
        }
        Err(e) => Err(fail(
            ExitCode::GeneralFailure,
            format!("could not check destination `{}`: {e}", out.display()),
        )),
    }
}

fn read_recording(file: &Path, limits: NativeReadLimits) -> Result<NativeRecording, AppError> {
    let invalid = |e: String| {
        fail(
            ExitCode::GeneralFailure,
            format!("invalid recording `{}`: {e}", file.display()),
        )
    };
    let mut input = File::open(file).map_err(|e| invalid(e.to_string()))?;
    let mut magic = [0u8; MAGIC.len()];
    if input.read_exact(&mut magic).is_err() || magic != MAGIC {
        return Err(invalid(
            "`record migrate` reads native (.nrec) recordings only".into(),
        ));
    }
    let prefixed = std::io::Cursor::new(magic).chain(input);
    NativeRecording::from_reader(prefixed, limits).map_err(|e| invalid(e.to_string()))
}

fn read_provenance(path: &Path) -> Result<LegacyProvenance, AppError> {
    let io = |e: std::io::Error| {
        fail(
            ExitCode::GeneralFailure,
            format!("could not read provenance `{}`: {e}", path.display()),
        )
    };
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io)?
        .take(MAX_PROVENANCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > MAX_PROVENANCE_BYTES {
        return Err(fail(
            ExitCode::GeneralFailure,
            format!(
                "provenance `{}` exceeds {MAX_PROVENANCE_BYTES} bytes",
                path.display()
            ),
        ));
    }
    LegacyProvenance::from_json(&bytes)
        .map_err(|e| fail(ExitCode::Compatibility, format!("migration refused: {e}")))
}
