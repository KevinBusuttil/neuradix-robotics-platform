//! Behavioural tests for deployment graph validation.
//!
//! Each invalid variant is derived from one valid baseline so the test asserts
//! that a *single* policy is what flips the deployment from valid to invalid.

use std::path::Path;

use neuradix_graph::{Severity, from_yaml};

fn path() -> &'static Path {
    Path::new("deployment.yaml")
}

/// A well-formed reference deployment: sensor -> planner -> safety -> actuator,
/// with Python confined to a best-effort perception stage.
fn valid_manifest() -> String {
    r#"
apiVersion: deploy.neuradix.io/v1alpha1
kind: RobotDeployment
metadata:
  name: reference-auv
spec:
  profile: marine-auv
  nodes:
    - name: main
      target: linux-aarch64
    - name: gpu
      target: linux-x86_64
  components:
    - name: sonar
      node: main
      executionClass: interactive
      provides: [range.v1]
    - name: perception
      node: gpu
      executionClass: batch-ai
      runtime: python
      requires: [range.v1]
      provides: [obstacles.v1]
    - name: planner
      node: main
      executionClass: interactive
      requires: [obstacles.v1]
      provides: [setpoint.v1]
    - name: safety
      node: main
      executionClass: deterministic
      role: safety
      requires: [setpoint.v1]
      provides: [thrust-cmd.v1]
    - name: thruster
      node: main
      executionClass: hard-real-time
      role: actuator
      requires: [thrust-cmd.v1]
  connections:
    - from: sonar
      to: perception
      contract: range.v1
    - from: perception
      to: planner
      contract: obstacles.v1
    - from: planner
      to: safety
      contract: setpoint.v1
    - from: safety
      to: thruster
      contract: thrust-cmd.v1
"#
    .to_owned()
}

fn report_codes(manifest: &str) -> Vec<String> {
    let report = from_yaml(manifest, path()).expect("well-formed YAML");
    report
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code.clone())
        .collect()
}

#[test]
fn valid_deployment_is_valid() {
    let report = from_yaml(&valid_manifest(), path()).unwrap();
    assert!(
        report.is_valid(),
        "expected valid, got issues: {:?}",
        report.issues
    );
    assert_eq!(report.error_count(), 0);
    assert!(report.identity.starts_with("sha256:"));
}

#[test]
fn identity_is_stable_across_reordering() {
    let a = from_yaml(&valid_manifest(), path()).unwrap();
    // Reverse the connection order; identity must be unchanged.
    let reordered = valid_manifest()
        .replace("range.v1\n    - from: perception", "PLACEHOLDER")
        .replace("PLACEHOLDER", "range.v1\n    - from: perception");
    let b = from_yaml(&reordered, path()).unwrap();
    assert_eq!(a.identity, b.identity);
}

#[test]
fn python_on_deterministic_path_is_rejected() {
    let manifest = valid_manifest().replace(
        "      executionClass: batch-ai\n      runtime: python",
        "      executionClass: deterministic\n      runtime: python",
    );
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"python-in-deterministic-path".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn python_feeding_deterministic_consumer_is_rejected() {
    // Make the planner deterministic so the Python perception stage feeds it.
    let manifest = valid_manifest().replace(
        "      executionClass: interactive\n      requires: [obstacles.v1]",
        "      executionClass: deterministic\n      requires: [obstacles.v1]",
    );
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"python-feeds-deterministic-path".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn actuator_bypassing_safety_is_rejected() {
    // Wire the planner directly to the thruster, bypassing safety.
    let manifest = valid_manifest().replace(
        "    - from: planner\n      to: safety\n      contract: setpoint.v1",
        "    - from: planner\n      to: thruster\n      contract: thrust-cmd.v1",
    );
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"actuator-authority-bypass".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn actuator_without_safety_authority_is_rejected() {
    // Drop the safety component's role so no Safety authority remains.
    let manifest = valid_manifest().replace("      role: safety\n", "");
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"missing-safety".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn unknown_node_placement_is_rejected() {
    let manifest = valid_manifest().replace("      node: gpu", "      node: nonexistent");
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"unknown-node".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn provider_mismatch_is_rejected() {
    // sonar advertises range.v1 but the connection claims a contract it does
    // not provide.
    let manifest = valid_manifest().replace(
        "    - from: sonar\n      to: perception\n      contract: range.v1",
        "    - from: sonar\n      to: perception\n      contract: depth.v1",
    );
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"provider-mismatch".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn unresolved_requirement_is_rejected() {
    // Remove the connection that feeds the planner.
    let manifest = valid_manifest().replace(
        "    - from: perception\n      to: planner\n      contract: obstacles.v1\n",
        "",
    );
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"unresolved-requirement".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn unknown_endpoint_is_rejected() {
    let manifest = valid_manifest().replace(
        "      to: perception\n      contract: range.v1",
        "      to: ghost\n      contract: range.v1",
    );
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"unknown-endpoint".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn cycle_is_rejected() {
    // Add a back-edge planner <- safety by making safety provide setpoint.v1
    // back to the planner, forming a cycle planner -> safety -> planner.
    let manifest = valid_manifest()
        .replace(
            "      role: safety\n      requires: [setpoint.v1]\n      provides: [thrust-cmd.v1]",
            "      role: safety\n      requires: [setpoint.v1]\n      provides: [thrust-cmd.v1, loop.v1]",
        )
        .replace(
            "      requires: [obstacles.v1]\n      provides: [setpoint.v1]",
            "      requires: [obstacles.v1, loop.v1]\n      provides: [setpoint.v1]",
        )
        .replace(
            "    - from: safety\n      to: thruster\n      contract: thrust-cmd.v1",
            "    - from: safety\n      to: thruster\n      contract: thrust-cmd.v1\n    - from: safety\n      to: planner\n      contract: loop.v1",
        );
    let codes = report_codes(&manifest);
    assert!(
        codes.contains(&"prohibited-cycle".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn missing_api_version_and_kind_are_rejected() {
    let manifest = r#"
metadata:
  name: bare
spec:
  components: []
"#;
    let codes = report_codes(manifest);
    assert!(
        codes.contains(&"missing-field".to_owned()),
        "codes: {codes:?}"
    );
}

#[test]
fn malformed_yaml_is_a_parse_error() {
    let err = from_yaml("this: : : not yaml", path()).unwrap_err();
    assert!(err.to_string().contains("failed to parse"));
}
