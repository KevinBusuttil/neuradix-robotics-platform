//! Graph commands: validate a deployment manifest.

use std::path::Path;

use neuradix_graph::{GraphError, GraphReport, Severity, load_file};
use serde_json::{Value, json};

use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

/// `neuradix graph validate <file>`
///
/// Validates a deployment manifest's topology and policy offline. A manifest
/// with any error-severity issue fails with [`ExitCode::DeploymentValidation`]
/// (10); warnings alone succeed. The content-addressed deployment identity is
/// always reported so a valid deployment can be pinned.
pub fn validate(file: &Path) -> Result<Outcome, AppError> {
    let report = load_file(file).map_err(map_graph_error)?;
    let data = report_to_json(&report);

    if report.is_valid() {
        let warnings = report
            .issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .map(|i| format!("{} [{}]: {}", i.path, i.code, i.message))
            .collect();
        Ok(Outcome::with_warnings(data, warnings))
    } else {
        Err(AppError::message(
            ExitCode::DeploymentValidation,
            format!(
                "deployment `{}` failed validation with {} error(s)",
                file.display(),
                report.error_count()
            ),
        )
        .with_data(data))
    }
}

fn report_to_json(report: &GraphReport) -> Value {
    let issues: Vec<Value> = report
        .issues
        .iter()
        .map(|i| {
            json!({
                "severity": i.severity.as_str(),
                "code": i.code,
                "path": i.path,
                "message": i.message,
            })
        })
        .collect();

    json!({
        "identity": report.identity,
        "valid": report.is_valid(),
        "errors": report.error_count(),
        "warnings": report.warning_count(),
        "issues": issues,
    })
}

/// Map a [`GraphError`] (I/O or parse failure) to an [`AppError`].
fn map_graph_error(err: GraphError) -> AppError {
    match &err {
        GraphError::Parse { .. } => {
            AppError::message(ExitCode::DeploymentValidation, err.to_string())
        }
        GraphError::Io { .. } => AppError::message(ExitCode::GeneralFailure, err.to_string()),
    }
}
