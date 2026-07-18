//! The `explain` command: reconstruct a command's causal lineage from a recording.

use std::path::Path;

use neuradix_safety::{CommandLineage, LINEAGE_CHANNEL};
use serde_json::{Value, json};

use crate::app::record::load;
use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

/// `neuradix explain command <recording> --at <nanos>`
///
/// Selects the command-lineage record nearest `at_nanos` and renders its causal
/// chain: originating sensor input -> requested command -> authority/constraint
/// outcome -> applied value.
pub fn command(file: &Path, at_nanos: i128) -> Result<Outcome, AppError> {
    let recording = load(file)?;

    // Collect lineage records from the well-known lineage channel(s).
    let lineage_channels: Vec<u16> = recording
        .manifest()
        .channels
        .iter()
        .filter(|c| c.name == LINEAGE_CHANNEL)
        .map(|c| c.id)
        .collect();

    let mut entries: Vec<CommandLineage> = Vec::new();
    for channel in lineage_channels {
        for record in recording.records_for(channel) {
            let lineage = CommandLineage::from_json_bytes(&record.payload).map_err(|e| {
                AppError::message(
                    ExitCode::GeneralFailure,
                    format!("corrupt command-lineage record: {e}"),
                )
            })?;
            entries.push(lineage);
        }
    }

    if entries.is_empty() {
        return Err(AppError::message(
            ExitCode::GeneralFailure,
            format!(
                "recording `{}` contains no command lineage (channel `{LINEAGE_CHANNEL}`)",
                file.display()
            ),
        ));
    }

    // Select the lineage record closest in time to the requested instant.
    let chosen = entries
        .iter()
        .min_by_key(|e| (e.at_nanos - at_nanos).unsigned_abs())
        .expect("entries is non-empty");

    let chain = vec![
        json!({
            "stage": "sensor",
            "source": chosen.origin.source,
            "quantity": chosen.origin.quantity,
            "value": chosen.origin.value,
            "unit": chosen.origin.unit,
        }),
        json!({ "stage": "control", "requested": chosen.requested }),
        json!({
            "stage": "authority-and-constraints",
            "outcome": chosen.outcome,
            "actedRules": chosen.acted_rules,
            "rejectReason": chosen.reject_reason,
        }),
        json!({ "stage": "applied", "value": chosen.applied }),
    ];

    let data = json!({
        "requestedAtNanos": at_nanos,
        "command": {
            "trace": chosen.trace,
            "atNanos": chosen.at_nanos,
            "clockDomain": chosen.clock_domain,
            "holder": chosen.holder,
            "capability": chosen.capability,
            "outcome": chosen.outcome,
            "applied": chosen.applied,
        },
        "lineage": Value::Array(chain),
    });

    Ok(Outcome::new(data))
}
