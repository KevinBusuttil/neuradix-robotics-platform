//! Deterministic canonicalization and content-addressed schema identity.
//!
//! Two contracts that are *logically* equivalent must produce the same schema
//! identity regardless of irrelevant formatting, key order or field-declaration
//! order, and regardless of whether a duration was written as `100ms` or
//! `0.1s`. This module defines the canonical form and the hash over it.
//!
//! ## What the identity covers
//!
//! The identity is over the normative *interface*: `apiVersion`, `kind`,
//! `metadata` (namespace/name/version), the payload fields (name, type, unit)
//! and the semantics (frame, clock domain, authoritative timestamp, maximum age
//! normalised to nanoseconds).
//!
//! It deliberately excludes:
//! - `spec.description` — non-normative human documentation; editing it must not
//!   change identity;
//! - `spec.delivery` — a queueing/QoS policy, not part of the wire schema.
//!
//! The rationale is recorded in
//! `docs/rfcs/RFC-0002-Contract-Format-and-Code-Generation.md`.
//!
//! ## Canonical form and hash
//!
//! The canonical form is UTF-8 JSON with:
//! - object keys sorted lexicographically;
//! - payload fields sorted by field name (so declaration order is irrelevant);
//! - the maximum age represented as an integer number of nanoseconds;
//! - no insignificant whitespace.
//!
//! The identity is `sha256:<lowercase-hex>` over the canonical bytes.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::model::Contract;

/// A content-addressed schema identity, e.g. `sha256:1a2b...`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SchemaId(String);

impl SchemaId {
    /// The hash algorithm used for schema identities.
    pub const ALGORITHM: &'static str = "sha256";

    /// The full identity string including the algorithm prefix.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse a `sha256:<64 lowercase hex>` identity string.
    ///
    /// Returns `None` if the prefix is missing or the digest is not exactly 64
    /// lowercase hexadecimal characters.
    pub fn parse(s: &str) -> Option<SchemaId> {
        let hex = s.strip_prefix("sha256:")?;
        let valid = hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| SchemaId(s.to_owned()))
    }

    /// The hex digest without the `sha256:` prefix.
    pub fn digest_hex(&self) -> &str {
        self.0.strip_prefix("sha256:").unwrap_or(&self.0)
    }
}

impl std::fmt::Display for SchemaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Build the canonical JSON value used for identity (already normalised, but not
/// necessarily key-sorted; [`canonical_bytes`] performs the final sort).
fn identity_value(contract: &Contract) -> Value {
    let mut fields: Vec<Value> = contract
        .spec
        .payload
        .fields
        .iter()
        .map(|f| {
            let mut obj = Map::new();
            obj.insert("name".to_owned(), Value::String(f.name.clone()));
            obj.insert(
                "type".to_owned(),
                Value::String(f.ty.as_contract_str().to_owned()),
            );
            if let Some(unit) = &f.unit {
                obj.insert("unit".to_owned(), Value::String(unit.clone()));
            }
            Value::Object(obj)
        })
        .collect();
    // Sort fields by name so authored declaration order does not affect identity.
    fields.sort_by(|a, b| {
        let an = a.get("name").and_then(Value::as_str).unwrap_or_default();
        let bn = b.get("name").and_then(Value::as_str).unwrap_or_default();
        an.cmp(bn)
    });

    let sem = &contract.spec.semantics;
    // Represent the normalised maximum age as a decimal string of nanoseconds.
    // A string avoids any dependence on JSON number representation (u128 vs f64)
    // and keeps the identity exact and deterministic for any magnitude.
    let maximum_age_nanos = sem.maximum_age.as_nanos().to_string();

    json!({
        "apiVersion": contract.api_version,
        "kind": contract.kind.as_str(),
        "metadata": {
            "namespace": contract.metadata.namespace,
            "name": contract.metadata.name,
            "version": contract.metadata.version.to_string(),
        },
        "payload": { "fields": fields },
        "semantics": {
            "frame": sem.frame,
            "clockDomain": sem.clock_domain.as_str(),
            "authoritativeTimestamp": sem.authoritative_timestamp,
            "maximumAgeNanos": maximum_age_nanos,
        },
    })
}

/// Recursively rewrite a JSON value so that every object's keys are sorted.
fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = Map::new();
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            for (k, v) in entries {
                sorted.insert(k, sort_keys(v));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

/// The canonical UTF-8 JSON bytes for a contract's schema identity.
///
/// This is stable across process runs and platforms for a given contract.
pub fn canonical_bytes(contract: &Contract) -> Vec<u8> {
    let canonical = sort_keys(identity_value(contract));
    // `serde_json::to_vec` on a value with only sorted objects and no floats is
    // deterministic and whitespace-free.
    serde_json::to_vec(&canonical).expect("canonical JSON serialization cannot fail")
}

/// The content-addressed schema identity for a contract.
pub fn schema_identity(contract: &Contract) -> SchemaId {
    let bytes = canonical_bytes(contract);
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let hex = to_hex(&digest);
    SchemaId(format!("sha256:{hex}"))
}

/// Lowercase hex encoding without external dependencies.
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
