//! Graph commands: validate a deployment manifest.

use std::path::Path;

use neuradix_graph::{
    ContractRegistry, GraphError, GraphReport, RegistryError, Severity, from_file,
    validate as validate_graph, validate_with_registry,
};
use serde_json::{Value, json};

use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

/// `neuradix graph validate <file> [--contracts <dir>]`
///
/// Validates a deployment manifest's topology and policy offline. When
/// `contracts` is given, every wired contract reference is additionally
/// resolved to a registered schema and the resolved identities are reported. A
/// manifest with any error-severity issue fails with
/// [`ExitCode::DeploymentValidation`] (10); warnings alone succeed. The
/// content-addressed deployment identity is always reported.
pub fn validate(file: &Path, contracts: Option<&Path>) -> Result<Outcome, AppError> {
    let raw = from_file(file).map_err(map_graph_error)?;

    let report = match contracts {
        Some(dir) => {
            let registry = ContractRegistry::load_dir(dir).map_err(map_registry_error)?;
            validate_with_registry(&raw, &registry)
        }
        None => validate_graph(&raw),
    };

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

    let resolved: Vec<Value> = report
        .resolved
        .iter()
        .map(|r| {
            json!({
                "reference": r.reference,
                "identifier": r.identifier,
                "version": r.version,
                "schemaId": r.schema_id,
            })
        })
        .collect();

    json!({
        "identity": report.identity,
        "valid": report.is_valid(),
        "errors": report.error_count(),
        "warnings": report.warning_count(),
        "issues": issues,
        "resolved": resolved,
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

/// Map a [`RegistryError`] (loading the contract registry) to an [`AppError`].
/// A malformed contract is a contract-validation failure; a missing directory
/// or a duplicate is a general failure.
fn map_registry_error(err: RegistryError) -> AppError {
    match &err {
        RegistryError::Contract { .. } => {
            AppError::message(ExitCode::ContractValidation, err.to_string())
        }
        RegistryError::Io { .. } | RegistryError::Duplicate { .. } => {
            AppError::message(ExitCode::GeneralFailure, err.to_string())
        }
    }
}
