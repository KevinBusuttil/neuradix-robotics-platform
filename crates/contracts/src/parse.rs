//! Raw deserialization of authored YAML contract documents.
//!
//! These types mirror the authored YAML shape as closely as possible. Every
//! field is optional so that missing required fields become precise validation
//! issues (see [`crate::validate`]) rather than opaque deserializer errors.
//! Unknown fields are tolerated to allow forward-compatible minor additions.

use std::path::Path;

use serde::Deserialize;

use crate::error::{ContractError, Result};

/// A raw, unvalidated contract document.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawContract {
    /// `apiVersion`
    pub api_version: Option<String>,
    /// `kind`
    pub kind: Option<String>,
    /// `metadata`
    pub metadata: Option<RawMetadata>,
    /// `spec`
    pub spec: Option<RawSpec>,
}

/// Raw metadata block.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMetadata {
    /// `metadata.namespace`
    pub namespace: Option<String>,
    /// `metadata.name`
    pub name: Option<String>,
    /// `metadata.version`
    pub version: Option<String>,
}

/// Raw spec block.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSpec {
    /// `spec.description`
    pub description: Option<String>,
    /// `spec.payload`
    pub payload: Option<RawPayload>,
    /// `spec.semantics`
    pub semantics: Option<RawSemantics>,
    /// `spec.delivery`
    pub delivery: Option<RawDelivery>,
}

/// Raw payload block.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPayload {
    /// `spec.payload.type`
    #[serde(rename = "type")]
    pub ty: Option<String>,
    /// `spec.payload.fields`, kept as an ordered mapping to preserve authored
    /// field declaration order.
    pub fields: Option<serde_yaml::Mapping>,
}

/// Raw semantics block.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSemantics {
    /// `spec.semantics.frame`
    pub frame: Option<String>,
    /// `spec.semantics.clockDomain`
    pub clock_domain: Option<String>,
    /// `spec.semantics.authoritativeTimestamp`
    pub authoritative_timestamp: Option<String>,
    /// `spec.semantics.maximumAge`
    pub maximum_age: Option<String>,
}

/// Raw delivery block.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDelivery {
    /// `spec.delivery.capacity`
    pub capacity: Option<u64>,
    /// `spec.delivery.overflow`
    pub overflow: Option<String>,
}

/// Parse a raw contract from a YAML string. `path` is used only for diagnostics.
pub fn from_yaml_str(source: &str, path: &Path) -> Result<RawContract> {
    serde_yaml::from_str(source).map_err(|source| ContractError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Read and parse a raw contract from a file on disk.
pub fn from_file(path: &Path) -> Result<RawContract> {
    let source = std::fs::read_to_string(path).map_err(|source| ContractError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    from_yaml_str(&source, path)
}
