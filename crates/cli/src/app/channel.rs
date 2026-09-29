//! `neuradix channel ...`: channel-manifest generation, verification and
//! board-table emission from an explicit binding specification (WP-A02.4).
//!
//! All manifest rules come from `neuradix_contracts::channel` (collision,
//! reserved-ID, name, codec, identity-format, length and digest checks) and the
//! board table from `neuradix_embedded_codegen::generate_channel_table`. This
//! module only reads bounded inputs, resolves contracts, compares and publishes.
//!
//! ## Binding specification
//!
//! ```yaml
//! apiVersion: channels.neuradix.io/v1alpha1
//! kind: ChannelBindings
//! channels:
//!   - compactId: 1
//!     name: vehicle-depth
//!     contract: ../contracts/navigation/vehicle-depth.yaml
//! ```
//!
//! Compact IDs and names are explicit; nothing is assigned or renumbered.
//! Contract paths are resolved relative to the specification's directory. The
//! codec, schema ID, wire ID and wire length are always derived from the
//! resolved contract; identity fields in a specification are unknown fields
//! and are rejected. One contract may back several channels.
//!
//! ## Verification levels
//!
//! - **Self-consistent**: the manifest parses, its version and fields are
//!   known, its entries obey the manifest rules and its declared digest is the
//!   digest of its entries. This says nothing about which contracts the entries
//!   describe: anyone can recompute a valid digest for wrong bindings.
//! - **Matches bindings**: additionally, every compact ID, name, codec, schema
//!   ID, wire ID and length equals what the binding specification and its
//!   contracts produce, with no missing or extra entries.
//!
//! Neither level authenticates a peer or establishes a link session.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use neuradix_contracts::layout::WireLayout;
use neuradix_contracts::{ChannelEntry, ChannelManifest, validate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::publish::{
    DestinationError, Published, ReadError, check_new_destination, leftover_warning, publish_new,
    read_bounded,
};
use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

/// Binding specification `apiVersion`.
pub const BINDINGS_API_VERSION: &str = "channels.neuradix.io/v1alpha1";
/// Binding specification `kind`.
pub const BINDINGS_KIND: &str = "ChannelBindings";
/// Largest binding specification, in bytes.
pub const MAX_BINDINGS_BYTES: u64 = 64 << 10;
/// Largest manifest document, in bytes.
pub const MAX_MANIFEST_BYTES: u64 = 1 << 20;
/// Largest single contract source, in bytes.
pub const MAX_CONTRACT_BYTES: u64 = 64 << 10;
/// Largest total of distinct contract sources read for one specification.
pub const MAX_TOTAL_CONTRACT_BYTES: u64 = 4 << 20;
/// Most channels in one specification or manifest.
pub const MAX_CHANNELS: usize = 256;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BindingSpec {
    api_version: String,
    kind: String,
    channels: Vec<Binding>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Binding {
    compact_id: u16,
    name: String,
    contract: PathBuf,
}

fn fail(exit: ExitCode, message: impl Into<String>) -> AppError {
    AppError::message(exit, message)
}

fn read(path: &Path, limit: u64, what: &str) -> Result<Vec<u8>, AppError> {
    read_bounded(path, limit).map_err(|e| match e {
        ReadError::Io(e) => fail(
            ExitCode::GeneralFailure,
            format!("could not read {what} `{}`: {e}", path.display()),
        ),
        ReadError::TooLarge { limit } => fail(
            ExitCode::GeneralFailure,
            format!("{what} `{}` exceeds {limit} bytes", path.display()),
        ),
    })
}

/// A resolved binding specification.
struct Resolved {
    manifest: ChannelManifest,
    layouts: BTreeMap<u16, WireLayout>,
    contracts: Vec<PathBuf>,
    contract_bytes: u64,
}

/// Read the specification and its contracts within the limits and build the
/// expected manifest with the library's rules.
fn resolve(spec_path: &Path) -> Result<Resolved, AppError> {
    let bytes = read(spec_path, MAX_BINDINGS_BYTES, "binding specification")?;
    let invalid = |e: String| {
        fail(
            ExitCode::Compatibility,
            format!(
                "invalid binding specification `{}`: {e}",
                spec_path.display()
            ),
        )
    };
    let text = std::str::from_utf8(&bytes).map_err(|e| invalid(e.to_string()))?;
    let spec: BindingSpec = serde_yaml::from_str(text).map_err(|e| invalid(e.to_string()))?;
    if spec.api_version != BINDINGS_API_VERSION || spec.kind != BINDINGS_KIND {
        return Err(invalid(format!(
            "expected apiVersion `{BINDINGS_API_VERSION}` and kind `{BINDINGS_KIND}`"
        )));
    }
    if spec.channels.len() > MAX_CHANNELS {
        return Err(invalid(format!(
            "{} channels exceed the limit of {MAX_CHANNELS}",
            spec.channels.len()
        )));
    }
    let base = crate::app::publish::parent(spec_path);
    let mut by_path: BTreeMap<PathBuf, WireLayout> = BTreeMap::new();
    let mut total = 0u64;
    let mut entries = Vec::with_capacity(spec.channels.len());
    let mut layouts = BTreeMap::new();
    for binding in &spec.channels {
        let path = base.join(&binding.contract);
        let layout = match by_path.get(&path) {
            Some(layout) => layout.clone(),
            None => {
                let source = read(&path, MAX_CONTRACT_BYTES, "contract")?;
                total += source.len() as u64;
                if total > MAX_TOTAL_CONTRACT_BYTES {
                    return Err(fail(
                        ExitCode::GeneralFailure,
                        format!("contracts exceed {MAX_TOTAL_CONTRACT_BYTES} bytes in total"),
                    ));
                }
                let contract_error = |e: String| {
                    fail(
                        ExitCode::ContractValidation,
                        format!("contract `{}`: {e}", path.display()),
                    )
                };
                let text =
                    std::str::from_utf8(&source).map_err(|e| contract_error(e.to_string()))?;
                let contract = validate::from_yaml_str(text, &path)
                    .map_err(|e| contract_error(e.to_string()))?;
                let layout = WireLayout::for_contract(&contract)
                    .map_err(|e| contract_error(e.to_string()))?;
                by_path.insert(path.clone(), layout.clone());
                layout
            }
        };
        entries.push(ChannelEntry::for_layout(
            binding.compact_id,
            binding.name.clone(),
            &layout,
        ));
        layouts.insert(binding.compact_id, layout);
    }
    let manifest = ChannelManifest::new(entries).map_err(|e| invalid(e.to_string()))?;
    Ok(Resolved {
        manifest,
        layouts,
        contracts: by_path.into_keys().collect(),
        contract_bytes: total,
    })
}

fn summary(manifest: &ChannelManifest) -> Value {
    json!({
        "digest": manifest.digest(),
        "channels": manifest.channels().iter().map(|c| json!({
            "compactId": c.compact_id,
            "name": c.name,
            "codecId": c.codec_id,
            "schemaId": c.schema_id,
            "wireId": c.wire_id,
            "wireLen": c.wire_len,
        })).collect::<Vec<_>>(),
    })
}

fn limits() -> Value {
    json!({
        "maxBindingsBytes": MAX_BINDINGS_BYTES,
        "maxManifestBytes": MAX_MANIFEST_BYTES,
        "maxContractBytes": MAX_CONTRACT_BYTES,
        "maxTotalContractBytes": MAX_TOTAL_CONTRACT_BYTES,
        "maxChannels": MAX_CHANNELS,
    })
}

fn publish(out: &Path, contents: &str, tag: &str) -> Result<Published, AppError> {
    publish_new(out, tag, |mut file| {
        use std::io::Write;
        file.write_all(contents.as_bytes())
    })
    .map_err(|e| {
        fail(
            ExitCode::GeneralFailure,
            format!("could not publish `{}`: {e}", out.display()),
        )
    })
}

fn destination(out: &Path, inputs: &[&Path]) -> Result<(), AppError> {
    check_new_destination(out, inputs).map_err(|e| match e {
        DestinationError::Refused(m) => fail(ExitCode::InvalidUse, m),
        // Permission denied, a non-directory path component, ...: operational.
        DestinationError::Io(m) => fail(ExitCode::GeneralFailure, m),
    })?;
    let dir = crate::app::publish::parent(out);
    if !std::fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Err(fail(
            ExitCode::GeneralFailure,
            format!("destination directory `{}` does not exist", dir.display()),
        ));
    }
    Ok(())
}

/// `neuradix channel manifest <bindings> --out <manifest.json>`.
pub fn manifest(spec: &Path, out: &Path) -> Result<Outcome, AppError> {
    destination(out, &[spec])?;
    let resolved = resolve(spec)?;
    let mut inputs: Vec<&Path> = vec![spec];
    inputs.extend(resolved.contracts.iter().map(PathBuf::as_path));
    destination(out, &inputs)?;
    let published = publish(out, &resolved.manifest.to_json_pretty(), "channel-manifest")?;
    let mut data = summary(&resolved.manifest);
    data["file"] = json!(out.display().to_string());
    data["bytes"] = json!(published.bytes);
    data["manifestVersion"] = json!(neuradix_contracts::channel::CHANNEL_MANIFEST_VERSION);
    data["contractBytes"] = json!(resolved.contract_bytes);
    data["limits"] = limits();
    Ok(Outcome::with_warnings(data, leftover_warning(&published)))
}

/// `neuradix channel table <bindings> --out <table.rs>`.
pub fn table(spec: &Path, out: &Path) -> Result<Outcome, AppError> {
    destination(out, &[spec])?;
    let resolved = resolve(spec)?;
    let mut inputs: Vec<&Path> = vec![spec];
    inputs.extend(resolved.contracts.iter().map(PathBuf::as_path));
    destination(out, &inputs)?;
    let source = neuradix_embedded_codegen::generate_channel_table(&resolved.manifest);
    let published = publish(out, &source, "channel-table")?;
    let mut data = summary(&resolved.manifest);
    data["file"] = json!(out.display().to_string());
    data["bytes"] = json!(published.bytes);
    data["limits"] = limits();
    Ok(Outcome::with_warnings(data, leftover_warning(&published)))
}

/// `neuradix channel verify <manifest> [--bindings <spec>]`.
pub fn verify(manifest_path: &Path, bindings: Option<&Path>) -> Result<Outcome, AppError> {
    let bytes = read(manifest_path, MAX_MANIFEST_BYTES, "channel manifest")?;
    let text = std::str::from_utf8(&bytes).map_err(|e| {
        fail(
            ExitCode::Compatibility,
            format!("manifest `{}` is not UTF-8: {e}", manifest_path.display()),
        )
    })?;
    let manifest = ChannelManifest::parse(text).map_err(|e| {
        fail(
            ExitCode::Compatibility,
            format!(
                "manifest `{}` is not self-consistent: {e}",
                manifest_path.display()
            ),
        )
    })?;
    if manifest.channels().len() > MAX_CHANNELS {
        return Err(fail(
            ExitCode::Compatibility,
            format!("manifest has more than {MAX_CHANNELS} channels"),
        ));
    }
    let mut data = summary(&manifest);
    data["file"] = json!(manifest_path.display().to_string());
    data["selfConsistent"] = json!(true);
    data["limits"] = limits();
    let Some(spec) = bindings else {
        data["level"] = json!("self-consistent");
        data["matchesBindings"] = Value::Null;
        return Ok(Outcome::with_warnings(
            data,
            vec![
                "self-consistent only: the digest covers the entries but does not show \
                 they describe the intended contracts; pass --bindings to check that"
                    .into(),
            ],
        ));
    };
    let expected = resolve(spec)?;
    let mut problems = Vec::new();
    for want in expected.manifest.channels() {
        let id = want.compact_id;
        match manifest.resolve(id) {
            None => problems.push(format!("missing channel {id} `{}`", want.name)),
            Some(got) => {
                if got.name != want.name {
                    problems.push(format!(
                        "channel {id}: name `{}` differs from expected `{}`",
                        got.name, want.name
                    ));
                }
                if let Err(e) = manifest.verify_layout(id, &expected.layouts[&id]) {
                    problems.push(format!("channel {id}: {e}"));
                }
            }
        }
    }
    for got in manifest.channels() {
        if expected.manifest.resolve(got.compact_id).is_none() {
            problems.push(format!(
                "unexpected channel {} `{}`",
                got.compact_id, got.name
            ));
        }
    }
    // Entry-by-entry agreement implies equal digests; check the whole anyway.
    if problems.is_empty() && manifest != expected.manifest {
        problems.push("manifest differs from the expected manifest".into());
    }
    data["expectedDigest"] = json!(expected.manifest.digest());
    data["bindings"] = json!(spec.display().to_string());
    if !problems.is_empty() {
        data["level"] = json!("self-consistent");
        data["matchesBindings"] = json!(false);
        data["mismatches"] = json!(problems);
        return Err(AppError {
            exit: ExitCode::Compatibility,
            errors: problems,
            data,
        });
    }
    data["level"] = json!("matches-bindings");
    data["matchesBindings"] = json!(true);
    Ok(Outcome::new(data))
}
