//! Contract golden-file helpers.

use std::path::Path;

use neuradix_contracts::{generate_rust, validate};

/// Parse `contract_yaml` and return the generated Rust source.
///
/// # Panics
/// Panics if the contract is invalid or code generation fails.
pub fn generate_rust_source(contract_yaml: &str) -> String {
    let contract = validate::from_yaml_str(contract_yaml, Path::new("<golden>"))
        .expect("contract under test should be valid");
    generate_rust(&contract)
        .expect("code generation should succeed")
        .code
}

/// Assert that generating Rust from `contract_yaml` reproduces `expected`
/// byte-for-byte.
///
/// # Panics
/// Panics with a helpful message if the generated code has drifted from the
/// committed golden file.
pub fn assert_generated_matches(contract_yaml: &str, expected: &str) {
    let generated = generate_rust_source(contract_yaml);
    assert_eq!(
        generated, expected,
        "generated code differs from the committed golden file; \
         regenerate with `neuradix contract generate`"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
apiVersion: contracts.neuradix.io/v1alpha1
kind: StreamContract
metadata:
  namespace: io.neuradix.test
  name: sample-value
  version: 2.1.0
spec:
  description: A sample value
  payload:
    type: object
    fields:
      value: { type: float64, unit: m }
  semantics:
    frame: vehicle/base
    clockDomain: simulation
    authoritativeTimestamp: measurement
    maximumAge: 250ms
  delivery:
    capacity: 4
    overflow: drop-oldest
"#;

    #[test]
    fn generation_is_deterministic_and_helper_matches() {
        let a = generate_rust_source(YAML);
        let b = generate_rust_source(YAML);
        assert_eq!(a, b, "generation must be deterministic");
        assert!(a.contains("pub struct SampleValue"));
        assert_generated_matches(YAML, &a);
    }
}
