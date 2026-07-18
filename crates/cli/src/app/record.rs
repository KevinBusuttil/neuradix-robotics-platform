//! Recording commands: inspect and replay.

use std::path::Path;

use neuradix_record::{NativeRecording, replay_digest};
use serde_json::{Value, json};

use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

pub(crate) fn load(file: &Path) -> Result<NativeRecording, AppError> {
    let bytes = std::fs::read(file).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not read recording `{}`: {e}", file.display()),
        )
    })?;
    NativeRecording::from_bytes(&bytes).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("invalid recording `{}`: {e}", file.display()),
        )
    })
}

/// `neuradix record inspect <file>`
pub fn inspect(file: &Path) -> Result<Outcome, AppError> {
    let recording = load(file)?;
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
        "formatVersion": manifest.format_version,
        "writer": manifest.writer,
        "note": manifest.note,
        "seed": manifest.seed,
        "records": recording.records().len(),
        "digest": replay_digest(&recording),
        "channels": channels,
        "software": software,
    })))
}

/// `neuradix replay run <file> [--expect-digest <sha256:...>]`
pub fn replay_run(file: &Path, expect_digest: Option<&str>) -> Result<Outcome, AppError> {
    let recording = load(file)?;
    let digest = replay_digest(&recording);
    let data = json!({
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
