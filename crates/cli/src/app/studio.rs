//! Studio commands: headless inspection of a recording.

use std::path::Path;

use neuradix_safety::{CommandLineage, LINEAGE_CHANNEL};
use neuradix_studio::{Inspection, ScalarDecoder, ScalarSample, StudioError};
use serde_json::{Value, json};

use crate::app::record::load;
use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

/// `neuradix studio timeline <file>` — per-domain spans and channel statistics.
///
/// Works on any recording (native `.nrec` or MCAP); the timeline is the read
/// model a visualization layer consumes.
pub fn timeline(file: &Path) -> Result<Outcome, AppError> {
    let loaded = load(file)?;
    let studio = Inspection::new(loaded.recording());
    let tl = studio.timeline();

    let domains: Vec<Value> = tl
        .domains
        .iter()
        .map(|d| {
            json!({
                "domain": d.domain,
                "startNanos": d.start_nanos.to_string(),
                "endNanos": d.end_nanos.to_string(),
                "durationNanos": d.duration_nanos.to_string(),
                "messages": d.message_count,
            })
        })
        .collect();

    let channels: Vec<Value> = tl
        .channels
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "schemaId": c.schema_id,
                "clockDomain": c.clock_domain,
                "count": c.count,
                "firstNanos": c.first_nanos.map(|v| v.to_string()),
                "lastNanos": c.last_nanos.map(|v| v.to_string()),
                "rateHz": c.rate_hz,
                "minPayload": c.min_payload,
                "maxPayload": c.max_payload,
                "totalPayload": c.total_payload,
            })
        })
        .collect();

    Ok(Outcome::new(json!({
        "messages": tl.message_count,
        "channels": tl.channel_count,
        "domains": domains,
        "channelStats": channels,
    })))
}

/// `neuradix studio series <file> --field <name> [--channel <id>]`
///
/// Extracts a plottable scalar series from the **command-lineage** channel: the
/// one channel with a known, decodable encoding. `field` is one of `requested`,
/// `applied` or `sensor`. Without `--channel`, the lineage channel is located by
/// name.
pub fn series(file: &Path, field: &str, channel: Option<u16>) -> Result<Outcome, AppError> {
    let loaded = load(file)?;
    let recording = loaded.recording();
    let studio = Inspection::new(recording);

    let channel_id = match channel {
        Some(id) => id,
        None => recording
            .manifest()
            .channels
            .iter()
            .find(|c| c.name == LINEAGE_CHANNEL)
            .map(|c| c.id)
            .ok_or_else(|| {
                AppError::message(
                    ExitCode::GeneralFailure,
                    format!(
                        "no command-lineage channel (`{LINEAGE_CHANNEL}`) in recording; pass --channel"
                    ),
                )
            })?,
    };

    let series = studio
        .series(channel_id, field, &LineageDecoder)
        .map_err(map_studio_error)?;

    let points: Vec<Value> = series
        .points
        .iter()
        .map(|p| json!({ "nanos": p.nanos.to_string(), "value": p.value }))
        .collect();
    let stats = series.stats.map(|s| {
        json!({
            "count": s.count,
            "min": s.min,
            "max": s.max,
            "mean": s.mean,
            "first": s.first,
            "last": s.last,
        })
    });

    Ok(Outcome::new(json!({
        "channel": channel_id,
        "field": series.field,
        "clockDomain": series.domain,
        "count": series.points.len(),
        "stats": stats,
        "points": points,
    })))
}

/// A decoder for the platform's command-lineage payloads.
struct LineageDecoder;

impl ScalarDecoder for LineageDecoder {
    fn decode(&self, payload: &[u8]) -> Result<Vec<ScalarSample>, StudioError> {
        let lineage = CommandLineage::from_json_bytes(payload)
            .map_err(|e| StudioError::Decode(e.to_string()))?;
        Ok(vec![
            ScalarSample::new("requested", lineage.requested),
            ScalarSample::new("applied", lineage.applied),
            ScalarSample::new("sensor", lineage.origin.value),
        ])
    }
}

fn map_studio_error(err: StudioError) -> AppError {
    AppError::message(ExitCode::GeneralFailure, err.to_string())
}
