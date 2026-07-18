//! Validation: raw authored document -> validated [`Contract`] model.
//!
//! Validation collects every problem it can find in a single pass and returns
//! them together, so an author fixing a contract sees the complete list rather
//! than one error at a time.

use std::num::NonZeroU32;
use std::path::Path;

use crate::error::{ContractError, Result, ValidationIssue};
use crate::model::{
    ClockDomainRef, Contract, ContractKind, Delivery, Duration, Field, Metadata, OverflowPolicy,
    Payload, PrimitiveType, SUPPORTED_API_VERSION, Semantics, Spec,
};
use crate::parse::{self, RawContract};

/// Parse and validate a contract from a YAML string.
pub fn from_yaml_str(source: &str, path: &Path) -> Result<Contract> {
    let raw = parse::from_yaml_str(source, path)?;
    validate(raw, path)
}

/// Read, parse and validate a contract file from disk.
pub fn load_file(path: &Path) -> Result<Contract> {
    let raw = parse::from_file(path)?;
    validate(raw, path)
}

/// Validate an already-parsed raw contract.
pub fn validate(raw: RawContract, path: &Path) -> Result<Contract> {
    let mut issues: Vec<ValidationIssue> = Vec::new();

    let api_version = require_str(&raw.api_version, "apiVersion", &mut issues);
    if let Some(v) = &api_version
        && v.as_str() != SUPPORTED_API_VERSION
    {
        issues.push(ValidationIssue::new(
            "apiVersion",
            format!("unsupported apiVersion `{v}`; expected `{SUPPORTED_API_VERSION}`"),
        ));
    }

    let kind = validate_kind(&raw.kind, &mut issues);
    let metadata = validate_metadata(raw.metadata.as_ref(), &mut issues);
    let spec = validate_spec(raw.spec.as_ref(), &mut issues);

    if issues.is_empty() {
        // All `Some` because there were no issues.
        Ok(Contract {
            api_version: api_version.expect("validated"),
            kind: kind.expect("validated"),
            metadata: metadata.expect("validated"),
            spec: spec.expect("validated"),
        })
    } else {
        let name = best_effort_name(&raw, path);
        Err(ContractError::Invalid { name, issues })
    }
}

fn best_effort_name(raw: &RawContract, path: &Path) -> String {
    let ns = raw.metadata.as_ref().and_then(|m| m.namespace.clone());
    let nm = raw.metadata.as_ref().and_then(|m| m.name.clone());
    match (ns, nm) {
        (Some(ns), Some(nm)) => format!("{ns}/{nm}"),
        (None, Some(nm)) => nm,
        _ => path.display().to_string(),
    }
}

fn require_str(
    value: &Option<String>,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) -> Option<String> {
    match value {
        Some(s) if !s.trim().is_empty() => Some(s.clone()),
        Some(_) => {
            issues.push(ValidationIssue::new(path, "must not be empty"));
            None
        }
        None => {
            issues.push(ValidationIssue::new(path, "is required"));
            None
        }
    }
}

fn validate_kind(raw: &Option<String>, issues: &mut Vec<ValidationIssue>) -> Option<ContractKind> {
    let s = require_str(raw, "kind", issues)?;
    match s.as_str() {
        "StreamContract" => Some(ContractKind::StreamContract),
        other => {
            issues.push(ValidationIssue::new(
                "kind",
                format!("unsupported kind `{other}`; only `StreamContract` is supported"),
            ));
            None
        }
    }
}

fn validate_metadata(
    raw: Option<&crate::parse::RawMetadata>,
    issues: &mut Vec<ValidationIssue>,
) -> Option<Metadata> {
    let Some(raw) = raw else {
        issues.push(ValidationIssue::new("metadata", "is required"));
        return None;
    };
    let namespace = require_str(&raw.namespace, "metadata.namespace", issues);
    let name = require_str(&raw.name, "metadata.name", issues);

    let version = match require_str(&raw.version, "metadata.version", issues) {
        Some(v) => match semver::Version::parse(&v) {
            Ok(ver) => Some(ver),
            Err(e) => {
                issues.push(ValidationIssue::new(
                    "metadata.version",
                    format!("`{v}` is not a valid semantic version: {e}"),
                ));
                None
            }
        },
        None => None,
    };

    Some(Metadata {
        namespace: namespace?,
        name: name?,
        version: version?,
    })
}

fn validate_spec(
    raw: Option<&crate::parse::RawSpec>,
    issues: &mut Vec<ValidationIssue>,
) -> Option<Spec> {
    let Some(raw) = raw else {
        issues.push(ValidationIssue::new("spec", "is required"));
        return None;
    };

    let payload = validate_payload(raw.payload.as_ref(), issues);
    let semantics = validate_semantics(raw.semantics.as_ref(), issues);
    let delivery = validate_delivery(raw.delivery.as_ref(), issues);

    Some(Spec {
        description: raw.description.clone(),
        payload: payload?,
        semantics: semantics?,
        delivery: delivery?,
    })
}

fn validate_payload(
    raw: Option<&crate::parse::RawPayload>,
    issues: &mut Vec<ValidationIssue>,
) -> Option<Payload> {
    let Some(raw) = raw else {
        issues.push(ValidationIssue::new("spec.payload", "is required"));
        return None;
    };

    match &raw.ty {
        Some(t) if t == "object" => {}
        Some(other) => issues.push(ValidationIssue::new(
            "spec.payload.type",
            format!("unsupported payload type `{other}`; only `object` is supported"),
        )),
        None => issues.push(ValidationIssue::new("spec.payload.type", "is required")),
    }

    let Some(fields_map) = &raw.fields else {
        issues.push(ValidationIssue::new("spec.payload.fields", "is required"));
        return None;
    };
    if fields_map.is_empty() {
        issues.push(ValidationIssue::new(
            "spec.payload.fields",
            "must declare at least one field",
        ));
        return None;
    }

    let mut fields = Vec::with_capacity(fields_map.len());
    for (key, value) in fields_map {
        let Some(field_name) = key.as_str() else {
            issues.push(ValidationIssue::new(
                "spec.payload.fields",
                "field names must be strings",
            ));
            continue;
        };
        let base = format!("spec.payload.fields.{field_name}");

        if !is_valid_field_name(field_name) {
            issues.push(ValidationIssue::new(
                &base,
                "field name must start with a letter and contain only letters, digits or '_'",
            ));
        }

        let Some(field_map) = value.as_mapping() else {
            issues.push(ValidationIssue::new(
                &base,
                "must be an object with a `type`",
            ));
            continue;
        };

        let ty = match field_map.get("type").and_then(|v| v.as_str()) {
            Some(t) => match PrimitiveType::parse(t) {
                Some(pt) => Some(pt),
                None => {
                    issues.push(ValidationIssue::new(
                        format!("{base}.type"),
                        format!(
                            "unsupported type `{t}`; supported: {}",
                            PrimitiveType::ALL.join(", ")
                        ),
                    ));
                    None
                }
            },
            None => {
                issues.push(ValidationIssue::new(format!("{base}.type"), "is required"));
                None
            }
        };

        let unit = field_map
            .get("unit")
            .and_then(|v| v.as_str())
            .map(str::to_owned);

        if let Some(ty) = ty {
            fields.push(Field {
                name: field_name.to_owned(),
                ty,
                unit,
            });
        }
    }

    if fields.is_empty() {
        // Every field had a problem; the specific issues were already recorded.
        return None;
    }

    Some(Payload { fields })
}

fn validate_semantics(
    raw: Option<&crate::parse::RawSemantics>,
    issues: &mut Vec<ValidationIssue>,
) -> Option<Semantics> {
    let Some(raw) = raw else {
        issues.push(ValidationIssue::new("spec.semantics", "is required"));
        return None;
    };

    let frame = require_str(&raw.frame, "spec.semantics.frame", issues);
    let authoritative_timestamp = require_str(
        &raw.authoritative_timestamp,
        "spec.semantics.authoritativeTimestamp",
        issues,
    );

    let clock_domain = match require_str(&raw.clock_domain, "spec.semantics.clockDomain", issues) {
        Some(s) => match ClockDomainRef::parse(&s) {
            Some(d) => Some(d),
            None => {
                issues.push(ValidationIssue::new(
                    "spec.semantics.clockDomain",
                    format!(
                        "unsupported clock domain `{s}`; supported: {}",
                        ClockDomainRef::ALL.join(", ")
                    ),
                ));
                None
            }
        },
        None => None,
    };

    let maximum_age = match require_str(&raw.maximum_age, "spec.semantics.maximumAge", issues) {
        Some(s) => match Duration::parse(&s) {
            Some(d) => Some(d),
            None => {
                issues.push(ValidationIssue::new(
                    "spec.semantics.maximumAge",
                    format!("`{s}` is not a valid duration (e.g. `100ms`, `0.1s`, `2m`)"),
                ));
                None
            }
        },
        None => None,
    };

    Some(Semantics {
        frame: frame?,
        clock_domain: clock_domain?,
        authoritative_timestamp: authoritative_timestamp?,
        maximum_age: maximum_age?,
    })
}

fn validate_delivery(
    raw: Option<&crate::parse::RawDelivery>,
    issues: &mut Vec<ValidationIssue>,
) -> Option<Delivery> {
    let Some(raw) = raw else {
        issues.push(ValidationIssue::new("spec.delivery", "is required"));
        return None;
    };

    let capacity = match raw.capacity {
        Some(0) => {
            issues.push(ValidationIssue::new(
                "spec.delivery.capacity",
                "must be greater than zero (unbounded queues are prohibited)",
            ));
            None
        }
        Some(c) if c > u64::from(u32::MAX) => {
            issues.push(ValidationIssue::new(
                "spec.delivery.capacity",
                format!(
                    "`{c}` exceeds the maximum supported capacity ({})",
                    u32::MAX
                ),
            ));
            None
        }
        Some(c) => NonZeroU32::new(c as u32),
        None => {
            issues.push(ValidationIssue::new(
                "spec.delivery.capacity",
                "is required",
            ));
            None
        }
    };

    let overflow = match require_str(&raw.overflow, "spec.delivery.overflow", issues) {
        Some(s) => match OverflowPolicy::parse(&s) {
            Some(p) => Some(p),
            None => {
                issues.push(ValidationIssue::new(
                    "spec.delivery.overflow",
                    format!(
                        "unsupported overflow policy `{s}`; supported: {}",
                        OverflowPolicy::ALL.join(", ")
                    ),
                ));
                None
            }
        },
        None => None,
    };

    Some(Delivery {
        capacity: capacity?,
        overflow: overflow?,
    })
}

/// Whether `name` is a valid field name for code generation.
fn is_valid_field_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
