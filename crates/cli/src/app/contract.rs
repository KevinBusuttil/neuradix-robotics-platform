//! Contract commands: validate, inspect, hash and generate.

use std::path::{Path, PathBuf};

use neuradix_contracts::{Contract, ContractError, generate_rust, load_file, schema_identity};
use serde_json::{Value, json};

use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

/// The supported target languages for `contract generate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Language {
    /// Generate a host Rust module.
    Rust,
    /// Generate a `no_std` Rust module with fixed little-endian encode/decode.
    NostdRust,
    /// Generate an Arduino/C++ header with fixed little-endian encode/decode.
    Cpp,
}

/// `neuradix contract validate <file-or-directory>`
pub fn validate(path: &Path) -> Result<Outcome, AppError> {
    let files = collect_contract_files(path)?;
    if files.is_empty() {
        let data = json!({ "checked": 0, "valid": 0, "invalid": 0, "results": [] });
        return Ok(Outcome::with_warnings(
            data,
            vec![format!(
                "no contract files (*.yaml / *.yml) found under `{}`",
                path.display()
            )],
        ));
    }

    let mut results = Vec::with_capacity(files.len());
    let mut valid = 0usize;
    let mut invalid = 0usize;

    for file in &files {
        match load_file(file) {
            Ok(contract) => {
                valid += 1;
                results.push(json!({
                    "file": file.display().to_string(),
                    "contract": contract.identifier(),
                    "status": "valid",
                    "schemaId": schema_identity(&contract).as_str(),
                }));
            }
            Err(err) => {
                invalid += 1;
                results.push(json!({
                    "file": file.display().to_string(),
                    "status": "invalid",
                    "issues": error_issues(&err),
                }));
            }
        }
    }

    let data = json!({
        "checked": files.len(),
        "valid": valid,
        "invalid": invalid,
        "results": results,
    });

    if invalid > 0 {
        Err(AppError::message(
            ExitCode::ContractValidation,
            format!("{invalid} of {} contract(s) failed validation", files.len()),
        )
        .with_data(data))
    } else {
        Ok(Outcome::new(data))
    }
}

/// `neuradix contract inspect <file>`
pub fn inspect(file: &Path) -> Result<Outcome, AppError> {
    let contract = load_file(file).map_err(map_contract_error)?;
    Ok(Outcome::new(contract_to_json(&contract)))
}

/// `neuradix contract hash <file>`
pub fn hash(file: &Path) -> Result<Outcome, AppError> {
    let contract = load_file(file).map_err(map_contract_error)?;
    Ok(Outcome::new(json!({
        "contract": contract.identifier(),
        "schemaId": schema_identity(&contract).as_str(),
    })))
}

/// `neuradix contract generate <file> --language <rust|nostd-rust|cpp> --out-dir <dir>`
pub fn generate(file: &Path, language: Language, out_dir: &Path) -> Result<Outcome, AppError> {
    let contract = load_file(file).map_err(map_contract_error)?;

    // Each language produces (output file name, source, type name, label).
    let (file_name, code, type_name, label) = match language {
        Language::Rust => {
            let g = generate_rust(&contract).map_err(map_contract_error)?;
            (format!("{}.rs", g.module_name), g.code, g.type_name, "rust")
        }
        Language::NostdRust => {
            let g = neuradix_embedded_codegen::generate_nostd_rust(&contract)
                .map_err(map_codegen_error)?;
            (
                format!("{}.rs", g.module_name),
                g.code,
                g.type_name,
                "nostd-rust",
            )
        }
        Language::Cpp => {
            let g =
                neuradix_embedded_codegen::generate_cpp(&contract).map_err(map_codegen_error)?;
            (format!("{}.h", g.header_name), g.code, g.type_name, "cpp")
        }
    };

    std::fs::create_dir_all(out_dir).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!(
                "could not create output directory `{}`: {e}",
                out_dir.display()
            ),
        )
    })?;

    let target = out_dir.join(&file_name);
    std::fs::write(&target, &code).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not write `{}`: {e}", target.display()),
        )
    })?;

    Ok(Outcome::new(json!({
        "contract": contract.identifier(),
        "language": label,
        "type": type_name,
        "file": target.display().to_string(),
        "schemaId": schema_identity(&contract).as_str(),
    })))
}

/// Map an embedded-codegen error to an [`AppError`].
fn map_codegen_error(err: neuradix_embedded_codegen::CodegenError) -> AppError {
    AppError::message(ExitCode::ContractValidation, err.to_string())
}

fn contract_to_json(contract: &Contract) -> Value {
    let fields: Vec<Value> = contract
        .spec
        .payload
        .fields
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "type": f.ty.as_contract_str(),
                "unit": f.unit,
            })
        })
        .collect();

    json!({
        "apiVersion": contract.api_version,
        "kind": contract.kind.as_str(),
        "namespace": contract.metadata.namespace,
        "name": contract.metadata.name,
        "version": contract.metadata.version.to_string(),
        "description": contract.spec.description,
        "schemaId": schema_identity(contract).as_str(),
        "semantics": {
            "frame": contract.spec.semantics.frame,
            "clockDomain": contract.spec.semantics.clock_domain.as_str(),
            "authoritativeTimestamp": contract.spec.semantics.authoritative_timestamp,
            "maximumAgeNanos": contract.spec.semantics.maximum_age.as_nanos().to_string(),
        },
        "delivery": {
            "capacity": contract.spec.delivery.capacity.get(),
            "overflow": contract.spec.delivery.overflow.as_str(),
        },
        "fields": fields,
    })
}

/// Map a [`ContractError`] to an [`AppError`] with the appropriate exit code.
fn map_contract_error(err: ContractError) -> AppError {
    match &err {
        ContractError::Invalid { issues, .. } => {
            let messages: Vec<String> = issues.iter().map(|i| i.to_string()).collect();
            AppError {
                exit: ExitCode::ContractValidation,
                errors: messages,
                data: Value::Null,
            }
        }
        ContractError::Parse { .. } => {
            AppError::message(ExitCode::ContractValidation, err.to_string())
        }
        ContractError::Io { .. } => AppError::message(ExitCode::GeneralFailure, err.to_string()),
        ContractError::Unsupported { .. } => {
            AppError::message(ExitCode::GeneralFailure, err.to_string())
        }
    }
}

/// Build a JSON array of issues from a contract error, for per-file reporting.
fn error_issues(err: &ContractError) -> Value {
    match err {
        ContractError::Invalid { issues, .. } => Value::Array(
            issues
                .iter()
                .map(|i| json!({ "path": i.path, "message": i.message }))
                .collect(),
        ),
        other => Value::Array(vec![json!({ "path": "", "message": other.to_string() })]),
    }
}

/// Collect the contract files implied by `path` (a single file or a directory
/// searched recursively for `*.yaml` / `*.yml`).
fn collect_contract_files(path: &Path) -> Result<Vec<PathBuf>, AppError> {
    if !path.exists() {
        return Err(AppError::message(
            ExitCode::GeneralFailure,
            format!("path does not exist: `{}`", path.display()),
        ));
    }
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut files = Vec::new();
    collect_recursive(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), AppError> {
    let entries = std::fs::read_dir(dir).map_err(|e| {
        AppError::message(
            ExitCode::GeneralFailure,
            format!("could not read directory `{}`: {e}", dir.display()),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            AppError::message(
                ExitCode::GeneralFailure,
                format!("directory read error: {e}"),
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, out)?;
        } else if is_yaml(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml")
    )
}
