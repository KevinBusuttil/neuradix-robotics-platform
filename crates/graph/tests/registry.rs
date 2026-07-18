//! Tests for contract-registry resolution and registry-aware validation.

use std::path::{Path, PathBuf};

use neuradix_graph::{
    ContractRegistry, Resolution, Severity, from_yaml_str, validate_with_registry,
};

/// The authored standard contracts shipped in the repository.
fn standard_contracts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/standard")
}

fn registry() -> ContractRegistry {
    ContractRegistry::load_dir(&standard_contracts()).expect("standard contracts load")
}

#[test]
fn loads_the_standard_contracts() {
    let reg = registry();
    assert!(!reg.is_empty());
    // At least the five authored standard contracts.
    assert!(reg.len() >= 5, "expected >= 5 contracts, got {}", reg.len());
}

#[test]
fn resolves_a_pinned_reference_to_a_schema_identity() {
    let reg = registry();
    match reg.resolve("io.neuradix.navigation/vehicle-depth@1.0.0") {
        Resolution::Resolved(entry) => {
            assert_eq!(entry.identifier, "io.neuradix.navigation/vehicle-depth");
            assert_eq!(entry.version, "1.0.0");
            assert!(entry.schema_id.starts_with("sha256:"));
        }
        other => panic!("expected resolved, got {other:?}"),
    }
}

#[test]
fn resolves_an_unpinned_single_version_reference() {
    let reg = registry();
    // Only one version of vehicle-depth is registered, so the bare identifier
    // resolves unambiguously.
    assert!(matches!(
        reg.resolve("io.neuradix.navigation/vehicle-depth"),
        Resolution::Resolved(_)
    ));
}

#[test]
fn unknown_contract_does_not_resolve() {
    let reg = registry();
    assert_eq!(
        reg.resolve("io.neuradix.nowhere/ghost"),
        Resolution::UnknownContract
    );
}

#[test]
fn unknown_version_does_not_resolve() {
    let reg = registry();
    assert_eq!(
        reg.resolve("io.neuradix.navigation/vehicle-depth@9.9.9"),
        Resolution::UnknownVersion
    );
}

#[test]
fn malformed_references_are_rejected() {
    let reg = registry();
    assert_eq!(reg.resolve(""), Resolution::Malformed);
    assert_eq!(reg.resolve("@1.0.0"), Resolution::Malformed);
    assert_eq!(
        reg.resolve("io.neuradix.navigation/vehicle-depth@"),
        Resolution::Malformed
    );
}

#[test]
fn multiple_versions_are_ambiguous_unless_pinned() {
    // Author two versions of the same contract in a temp registry.
    let dir = std::env::temp_dir().join("neuradix-graph-registry-ambiguous");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for version in ["1.0.0", "2.0.0"] {
        let yaml = format!(
            concat!(
                "apiVersion: contracts.neuradix.io/v1alpha1\n",
                "kind: StreamContract\n",
                "metadata:\n",
                "  namespace: io.neuradix.test\n",
                "  name: widget\n",
                "  version: {version}\n",
                "spec:\n",
                "  description: test\n",
                "  payload:\n    type: object\n    fields:\n      x: {{ type: float64 }}\n",
                "  semantics:\n    frame: vehicle/base\n    clockDomain: monotonic\n",
                "    authoritativeTimestamp: measurement\n    maximumAge: 100ms\n",
                "  delivery:\n    capacity: 4\n    overflow: keep-latest\n",
            ),
            version = version
        );
        std::fs::write(dir.join(format!("widget-{version}.yaml")), yaml).unwrap();
    }

    let reg = ContractRegistry::load_dir(&dir).unwrap();
    match reg.resolve("io.neuradix.test/widget") {
        Resolution::Ambiguous(versions) => {
            assert_eq!(versions, vec!["1.0.0".to_owned(), "2.0.0".to_owned()]);
        }
        other => panic!("expected ambiguous, got {other:?}"),
    }
    // Pinning resolves it.
    assert!(matches!(
        reg.resolve("io.neuradix.test/widget@2.0.0"),
        Resolution::Resolved(_)
    ));

    let _ = std::fs::remove_dir_all(&dir);
}

/// A deployment wiring the real standard contracts by their pinned references.
fn reference_manifest() -> &'static str {
    r#"
apiVersion: deploy.neuradix.io/v1alpha1
kind: RobotDeployment
metadata:
  name: registry-demo
spec:
  nodes:
    - name: main
      target: linux-aarch64
  components:
    - name: planner
      node: main
      executionClass: interactive
      provides: [io.neuradix.control/depth-setpoint@1.0.0]
    - name: safety
      node: main
      executionClass: deterministic
      role: safety
      requires: [io.neuradix.control/depth-setpoint@1.0.0]
      provides: [io.neuradix.actuation/thrust-command@1.0.0]
    - name: thruster
      node: main
      executionClass: hard-real-time
      role: actuator
      requires: [io.neuradix.actuation/thrust-command@1.0.0]
  connections:
    - from: planner
      to: safety
      contract: io.neuradix.control/depth-setpoint@1.0.0
    - from: safety
      to: thruster
      contract: io.neuradix.actuation/thrust-command@1.0.0
"#
}

#[test]
fn registry_validation_resolves_every_wired_contract() {
    let raw = from_yaml_str(reference_manifest(), Path::new("registry-demo.yaml")).unwrap();
    let report = validate_with_registry(&raw, &registry());
    assert!(report.is_valid(), "issues: {:?}", report.issues);
    assert_eq!(report.resolved.len(), 2);
    assert!(
        report
            .resolved
            .iter()
            .all(|r| r.schema_id.starts_with("sha256:"))
    );
}

#[test]
fn registry_validation_flags_an_unregistered_reference() {
    let manifest = reference_manifest().replace(
        "io.neuradix.actuation/thrust-command@1.0.0",
        "io.neuradix.actuation/ghost@1.0.0",
    );
    let raw = from_yaml_str(&manifest, Path::new("registry-demo.yaml")).unwrap();
    let report = validate_with_registry(&raw, &registry());
    assert!(!report.is_valid());
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.severity == Severity::Error && i.code == "unknown-contract"),
        "issues: {:?}",
        report.issues
    );
}
