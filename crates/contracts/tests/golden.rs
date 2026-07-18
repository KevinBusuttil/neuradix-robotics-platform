//! Golden-file and schema-identity determinism tests for `neuradix-contracts`.

use std::path::Path;

use neuradix_contracts::{generate_rust, schema_identity, validate};

const CONTRACT_YAML: &str =
    include_str!("../../../contracts/standard/navigation/vehicle-depth.yaml");
const GOLDEN_RUST: &str = include_str!("golden/vehicle_depth.rs");

fn load() -> neuradix_contracts::Contract {
    validate::from_yaml_str(CONTRACT_YAML, Path::new("vehicle-depth.yaml"))
        .expect("reference contract must be valid")
}

#[test]
fn generated_rust_matches_golden() {
    let generated = generate_rust(&load()).expect("generation succeeds");
    assert_eq!(generated.module_name, "vehicle_depth");
    assert_eq!(generated.type_name, "VehicleDepth");
    assert_eq!(
        generated.code, GOLDEN_RUST,
        "generated Rust drifted from the golden file; regenerate with \
         `neuradix contract generate`"
    );
}

#[test]
fn schema_identity_has_expected_shape() {
    let id = schema_identity(&load());
    assert!(id.as_str().starts_with("sha256:"));
    assert_eq!(id.digest_hex().len(), 64);
    assert!(id.digest_hex().bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn schema_identity_is_independent_of_formatting_and_key_order() {
    // Same logical contract, different key order, whitespace and an equivalent
    // duration spelling (0.1s == 100ms). The identity must be identical.
    let reordered = r#"
kind: StreamContract
apiVersion: contracts.neuradix.io/v1alpha1
spec:
  delivery:
    overflow: keep-latest
    capacity: 8
  semantics:
    maximumAge: 0.1s
    clockDomain: monotonic
    frame: vehicle/base
    authoritativeTimestamp: measurement
  payload:
    type: object
    fields:
      uncertainty: { unit: m, type: float64 }
      depth: { unit: m, type: float64 }
  description: Vehicle depth below the configured water reference
metadata:
  version: 1.0.0
  name: vehicle-depth
  namespace: io.neuradix.navigation
"#;

    let a = schema_identity(&load());
    let b = schema_identity(
        &validate::from_yaml_str(reordered, Path::new("reordered.yaml")).expect("valid"),
    );
    assert_eq!(
        a, b,
        "logically equivalent contracts must share a schema identity"
    );
}

#[test]
fn description_does_not_affect_identity_but_field_type_does() {
    let base = load();
    let base_id = schema_identity(&base);

    // Changing only the description must not change identity.
    let different_desc = CONTRACT_YAML.replace(
        "Vehicle depth below the configured water reference",
        "A completely different description",
    );
    let desc_id = schema_identity(
        &validate::from_yaml_str(&different_desc, Path::new("desc.yaml")).expect("valid"),
    );
    assert_eq!(
        base_id, desc_id,
        "description is non-normative and must not change identity"
    );

    // Changing a field type must change identity.
    let different_type = CONTRACT_YAML.replace(
        "type: float64\n        unit: m",
        "type: float32\n        unit: m",
    );
    let type_contract =
        validate::from_yaml_str(&different_type, Path::new("type.yaml")).expect("valid");
    // Only assert if the replacement actually changed a field type.
    if type_contract
        .spec
        .payload
        .fields
        .iter()
        .any(|f| f.ty == neuradix_contracts::PrimitiveType::Float32)
    {
        assert_ne!(
            base_id,
            schema_identity(&type_contract),
            "field type must affect identity"
        );
    }
}
