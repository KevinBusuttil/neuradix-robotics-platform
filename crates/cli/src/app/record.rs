//! Recording commands: inspect, replay and export.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use neuradix_record::{
    MAGIC, MCAP_MAGIC, McapArchive, McapImportLimits, McapRecording, McapWriter, NativeRecording,
    Recording, replay_digest,
};
use serde_json::{Value, json};

use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

/// A recording loaded from either supported container.
pub(crate) struct Loaded {
    recording: Box<dyn Recording>,
    format: &'static str,
}

impl Loaded {
    /// Borrow the backend-neutral recording.
    pub(crate) fn recording(&self) -> &dyn Recording {
        self.recording.as_ref()
    }
}

/// Read a recording from disk, detecting the container format by magic bytes.
pub(crate) fn load(file: &Path) -> Result<Loaded, AppError> {
    let (mut input, magic) = open(file)?;

    let invalid = |e: neuradix_record::RecordError| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("invalid recording `{}`: {e}", file.display()),
        )
    };

    if magic == MCAP_MAGIC {
        let recording =
            McapRecording::from_reader(input, McapImportLimits::default()).map_err(invalid)?;
        Ok(Loaded {
            recording: Box::new(recording),
            format: "mcap",
        })
    } else if magic.starts_with(&MAGIC) {
        // Native-container resource hardening is separate from bounded MCAP import.
        let mut bytes = Vec::new();
        input
            .read_to_end(&mut bytes)
            .map_err(|e| invalid(e.into()))?;
        let recording = NativeRecording::from_bytes(&bytes).map_err(invalid)?;
        Ok(Loaded {
            recording: Box::new(recording),
            format: "native",
        })
    } else {
        Err(AppError::message(
            ExitCode::GeneralFailure,
            format!(
                "unrecognized recording `{}`: not a native (.nrec) or MCAP container",
                file.display()
            ),
        ))
    }
}

fn open(file: &Path) -> Result<(std::fs::File, [u8; 8]), AppError> {
    let io = |e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not read recording `{}`: {e}", file.display()),
        )
    };
    let mut input = std::fs::File::open(file).map_err(io)?;
    let mut magic = [0; 8];
    input.read_exact(&mut magic).map_err(io)?;
    input.seek(SeekFrom::Start(0)).map_err(io)?;
    Ok((input, magic))
}

/// `neuradix record inspect <file>` — works for native or MCAP recordings.
pub fn inspect(file: &Path) -> Result<Outcome, AppError> {
    let (input, magic) = open(file)?;
    if magic == MCAP_MAGIC {
        let archive =
            McapArchive::from_reader(input, McapImportLimits::default()).map_err(|e| {
                AppError::message(ExitCode::GeneralFailure, format!("invalid MCAP: {e}"))
            })?;
        let summary = archive.summary();
        let channels: Vec<Value> = summary
            .channels()
            .values()
            .map(|c| {
                json!({
                    "id": c.id, "name": c.topic, "schemaId": c.schema_id,
                    "messageEncoding": c.message_encoding, "metadata": c.metadata,
                    "records": archive.messages().iter().filter(|m| m.channel_id == c.id).count(),
                })
            })
            .collect();
        let schemas: Vec<Value> = summary
            .schemas()
            .values()
            .map(|s| {
                json!({
                    "id": s.id, "name": s.name, "encoding": s.encoding, "bytes": s.data.len(),
                })
            })
            .collect();
        let metadata: Vec<Value> = summary
            .metadata()
            .values()
            .map(|m| json!({"name": m.name, "entries": m.entries}))
            .collect();
        let mut output = json!({
            "format": "mcap", "profile": summary.header().profile, "writer": summary.header().library,
            "records": archive.messages().len(), "channels": channels, "schemas": schemas,
            "metadata": metadata, "auxiliaryRecords": archive.auxiliary().len(),
            "timestampSemantics": "raw log/publish nanoseconds; epoch is producer-defined",
            "inputBytes": summary.stats().input_bytes, "retainedBytes": archive.retained_bytes(),
        });
        // Preserve the historical digest only where the checked projection is
        // lossless. A foreign archive still inspects, with no legacy digest claim.
        output["digest"] = archive
            .try_into_recording()
            .ok()
            .map(|recording| json!(replay_digest(&recording)))
            .unwrap_or(Value::Null);
        return Ok(Outcome::new(output));
    }
    let loaded = load(file)?;
    let recording = loaded.recording();
    let manifest = recording.manifest();

    let channels: Vec<Value> = manifest
        .channels
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "schemaId": c.schema_id,
                "clockDomain": c.clock_domain,
                "records": recording.count_for(c.id),
            })
        })
        .collect();
    let software: Vec<Value> = manifest
        .software
        .iter()
        .map(|s| json!({ "name": s.name, "version": s.version }))
        .collect();

    Ok(Outcome::new(json!({
        "format": loaded.format,
        "formatVersion": manifest.format_version,
        "writer": manifest.writer,
        "note": manifest.note,
        "seed": manifest.seed,
        "records": recording.records().len(),
        "digest": replay_digest(recording),
        "channels": channels,
        "software": software,
    })))
}

/// `neuradix replay run <file> [--expect-digest <sha256:...>]` — native or MCAP.
pub fn replay_run(file: &Path, expect_digest: Option<&str>) -> Result<Outcome, AppError> {
    let loaded = load(file)?;
    let recording = loaded.recording();
    let digest = replay_digest(recording);
    let data = json!({
        "format": loaded.format,
        "records": recording.records().len(),
        "channels": recording.manifest().channels.len(),
        "digest": digest,
    });

    if let Some(expected) = expect_digest
        && expected != digest
    {
        return Err(AppError {
            exit: ExitCode::DeterminismMismatch,
            errors: vec![format!(
                "replay digest mismatch: expected `{expected}`, got `{digest}`"
            )],
            data,
        });
    }

    Ok(Outcome::new(data))
}

/// `neuradix record export <file> --out <path>` — re-encode any recording as MCAP.
///
/// The MCAP replay digest is identical to the source recording's, so exporting
/// preserves the legacy record digest. Foreign MCAP that cannot be represented
/// by the existing writer rejects explicitly; container support is not ROS decoding.
pub fn export(file: &Path, out: &Path) -> Result<Outcome, AppError> {
    let loaded = load(file)?;
    let recording = loaded.recording();
    let source_digest = replay_digest(recording);

    let mut writer = McapWriter::new(Vec::new(), recording.manifest()).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not start MCAP writer: {e}"),
        )
    })?;
    for record in recording.records() {
        writer
            .write_record(
                record.channel_id,
                record.sequence,
                record.timestamp,
                &record.payload,
            )
            .map_err(|e| {
                AppError::message(
                    ExitCode::GeneralFailure,
                    format!("could not encode MCAP: {e}"),
                )
            })?;
    }
    let bytes = writer.finish().map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not finish MCAP: {e}"),
        )
    })?;

    std::fs::write(out, &bytes).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not write `{}`: {e}", out.display()),
        )
    })?;

    Ok(Outcome::new(json!({
        "sourceFormat": loaded.format,
        "outputFormat": "mcap",
        "file": out.display().to_string(),
        "bytes": bytes.len(),
        "records": recording.records().len(),
        "digest": source_digest,
    })))
}
