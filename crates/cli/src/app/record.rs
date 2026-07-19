//! Recording commands: inspect, replay and export.

use std::path::Path;

use neuradix_record::{
    MAGIC, MCAP_MAGIC, McapRecording, McapWriter, NativeRecording, Recording, replay_digest,
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
    let bytes = std::fs::read(file).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not read recording `{}`: {e}", file.display()),
        )
    })?;

    let invalid = |e: neuradix_record::RecordError| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("invalid recording `{}`: {e}", file.display()),
        )
    };

    if bytes.starts_with(&MCAP_MAGIC) {
        let recording = McapRecording::from_bytes(&bytes).map_err(invalid)?;
        Ok(Loaded {
            recording: Box::new(recording),
            format: "mcap",
        })
    } else if bytes.starts_with(&MAGIC) {
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

/// `neuradix record inspect <file>` — works for native or MCAP recordings.
pub fn inspect(file: &Path) -> Result<Outcome, AppError> {
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
/// preserves behavioural identity; the output is readable by MCAP tooling
/// (Foxglove, ROS 2).
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
