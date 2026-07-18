//! Validation behaviour tests for `neuradix-contracts`.

use std::path::Path;

use neuradix_contracts::{ContractError, OverflowPolicy, PrimitiveType, validate};

fn parse(yaml: &str) -> Result<neuradix_contracts::Contract, ContractError> {
    validate::from_yaml_str(yaml, Path::new("<test>"))
}

const VALID: &str = r#"
apiVersion: contracts.neuradix.io/v1alpha1
kind: StreamContract
metadata:
  namespace: io.neuradix.navigation
  name: vehicle-depth
  version: 1.0.0
spec:
  payload:
    type: object
    fields:
      depth: { type: float64, unit: m }
  semantics:
    frame: vehicle/base
    clockDomain: monotonic
    authoritativeTimestamp: measurement
    maximumAge: 100ms
  delivery:
    capacity: 8
    overflow: keep-latest
"#;

#[test]
fn valid_contract_parses() {
    let c = parse(VALID).expect("valid");
    assert_eq!(c.metadata.namespace, "io.neuradix.navigation");
    assert_eq!(c.metadata.name, "vehicle-depth");
    assert_eq!(c.metadata.version.to_string(), "1.0.0");
    assert_eq!(c.spec.payload.fields.len(), 1);
    assert_eq!(c.spec.payload.fields[0].ty, PrimitiveType::Float64);
    assert_eq!(c.spec.payload.fields[0].unit.as_deref(), Some("m"));
    assert_eq!(c.spec.delivery.capacity.get(), 8);
    assert_eq!(c.spec.delivery.overflow, OverflowPolicy::KeepLatest);
    assert_eq!(c.spec.semantics.maximum_age.as_nanos(), 100_000_000);
}

#[test]
fn invalid_contract_collects_all_issues() {
    let bad = r#"
apiVersion: wrong/v1
kind: WeirdContract
metadata:
  namespace: io.neuradix.navigation
  name: bad
  version: not-a-semver
spec:
  payload:
    type: object
    fields:
      depth: { type: float128, unit: m }
  semantics:
    frame: vehicle/base
    clockDomain: warpspeed
    authoritativeTimestamp: measurement
    maximumAge: 100furlongs
  delivery:
    capacity: 0
    overflow: explode
"#;
    let err = parse(bad).expect_err("must be invalid");
    match err {
        ContractError::Invalid { issues, .. } => {
            let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
            // Every independent problem should be reported in one pass.
            for expected in [
                "apiVersion",
                "kind",
                "metadata.version",
                "spec.payload.fields.depth.type",
                "spec.semantics.clockDomain",
                "spec.semantics.maximumAge",
                "spec.delivery.capacity",
                "spec.delivery.overflow",
            ] {
                assert!(
                    paths.contains(&expected),
                    "missing issue for `{expected}`; got {paths:?}"
                );
            }
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn missing_required_sections_are_reported() {
    let err = parse("apiVersion: contracts.neuradix.io/v1alpha1\nkind: StreamContract\n")
        .expect_err("missing metadata and spec");
    let ContractError::Invalid { issues, .. } = err else {
        panic!("expected Invalid");
    };
    let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
    assert!(paths.contains(&"metadata"));
    assert!(paths.contains(&"spec"));
}

#[test]
fn unbounded_queue_is_rejected() {
    let bad = VALID.replace("capacity: 8", "capacity: 0");
    let err = parse(&bad).expect_err("zero capacity is invalid");
    assert!(err.to_string().contains("greater than zero"));
}
