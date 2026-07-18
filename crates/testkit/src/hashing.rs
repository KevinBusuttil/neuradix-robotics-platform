//! Deterministic schema-hashing helpers.

use std::path::Path;

use neuradix_contracts::{SchemaId, schema_identity, validate};

/// Compute the schema identity of `contract_yaml`.
///
/// # Panics
/// Panics if the contract is invalid.
pub fn schema_id_of(contract_yaml: &str) -> SchemaId {
    let contract = validate::from_yaml_str(contract_yaml, Path::new("<hash>"))
        .expect("contract under test should be valid");
    schema_identity(&contract)
}

/// Assert that every YAML string in `variants` yields the same schema identity,
/// i.e. that the variants are logically equivalent despite formatting or field
/// ordering differences.
///
/// # Panics
/// Panics if `variants` is empty or if any variant produces a different
/// identity from the first.
pub fn assert_schema_stable(variants: &[&str]) -> SchemaId {
    let (first, rest) = variants.split_first().expect("need at least one variant");
    let expected = schema_id_of(first);
    for (index, variant) in rest.iter().enumerate() {
        let actual = schema_id_of(variant);
        assert_eq!(
            actual,
            expected,
            "variant {} produced a different schema identity ({actual} != {expected})",
            index + 1
        );
    }
    expected
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = r#"
apiVersion: contracts.neuradix.io/v1alpha1
kind: StreamContract
metadata: { namespace: io.neuradix.test, name: x, version: 1.0.0 }
spec:
  payload: { type: object, fields: { a: { type: int32 }, b: { type: int32 } } }
  semantics: { frame: f, clockDomain: monotonic, authoritativeTimestamp: measurement, maximumAge: 1s }
  delivery: { capacity: 2, overflow: reject }
"#;
    const B_REORDERED: &str = r#"
kind: StreamContract
apiVersion: contracts.neuradix.io/v1alpha1
spec:
  delivery: { overflow: reject, capacity: 2 }
  semantics: { maximumAge: 1000ms, authoritativeTimestamp: measurement, clockDomain: monotonic, frame: f }
  payload: { type: object, fields: { b: { type: int32 }, a: { type: int32 } } }
metadata: { version: 1.0.0, name: x, namespace: io.neuradix.test }
"#;

    #[test]
    fn reordered_equivalent_contracts_share_identity() {
        assert_schema_stable(&[A, B_REORDERED]);
    }
}
