//! End-to-end tests for `neuradix graph validate`.

use std::process::Command;

use neuradix_testkit::cli_output::ParsedEnvelope;

const REFERENCE_DEPLOYMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/reference-auv/deployment.yaml"
);

const STANDARD_CONTRACTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../contracts/standard");

fn run(args: &[&str]) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_neuradix"))
        .args(args)
        .output()
        .expect("neuradix binary should run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn reference_deployment_validates_successfully() {
    let (stdout, code) = run(&["-o", "json", "graph", "validate", REFERENCE_DEPLOYMENT]);
    assert_eq!(code, 0, "reference deployment must validate");
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_well_formed()
        .assert_command("graph.validate")
        .assert_success();
    assert_eq!(env.data_field("valid").unwrap(), true);
    assert!(
        env.data_field("identity")
            .and_then(|v| v.as_str())
            .unwrap()
            .starts_with("sha256:")
    );
}

#[test]
fn reference_deployment_resolves_against_the_contract_registry() {
    let (stdout, code) = run(&[
        "-o",
        "json",
        "graph",
        "validate",
        REFERENCE_DEPLOYMENT,
        "--contracts",
        STANDARD_CONTRACTS,
    ]);
    assert_eq!(code, 0, "reference deployment must resolve");
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_command("graph.validate").assert_success();
    // Every wired contract reference resolved to a real schema identity.
    let resolved = env
        .data_field("resolved")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(resolved.len(), 4);
    assert!(resolved.iter().all(|r| {
        r["schemaId"]
            .as_str()
            .is_some_and(|s| s.starts_with("sha256:"))
    }));
}

#[test]
fn unresolved_contract_reference_exits_with_code_10() {
    let dir = std::env::temp_dir().join("neuradix-cli-graph-registry-it");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ghost-ref.yaml");
    // A well-formed deployment that references a contract absent from the
    // registry.
    std::fs::write(
        &path,
        concat!(
            "apiVersion: deploy.neuradix.io/v1alpha1\n",
            "kind: RobotDeployment\n",
            "metadata:\n  name: ghost-ref\n",
            "spec:\n",
            "  nodes:\n    - name: main\n      target: linux-aarch64\n",
            "  components:\n",
            "    - name: a\n      node: main\n      executionClass: interactive\n      provides: [io.neuradix.nowhere/ghost@1.0.0]\n",
            "    - name: b\n      node: main\n      executionClass: interactive\n      requires: [io.neuradix.nowhere/ghost@1.0.0]\n",
            "  connections:\n",
            "    - from: a\n      to: b\n      contract: io.neuradix.nowhere/ghost@1.0.0\n",
        ),
    )
    .unwrap();

    let (stdout, code) = run(&[
        "-o",
        "json",
        "graph",
        "validate",
        path.to_str().unwrap(),
        "--contracts",
        STANDARD_CONTRACTS,
    ]);
    assert_eq!(
        code, 10,
        "an unresolved contract reference must fail deployment validation"
    );
    ParsedEnvelope::parse(&stdout).unwrap().assert_failure();

    let _ = std::fs::remove_file(&path);
}

#[test]
fn invalid_deployment_exits_with_code_10() {
    let dir = std::env::temp_dir().join("neuradix-cli-graph-it");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bypass.yaml");
    // An actuator commanded directly by a non-Safety component.
    std::fs::write(
        &path,
        concat!(
            "apiVersion: deploy.neuradix.io/v1alpha1\n",
            "kind: RobotDeployment\n",
            "metadata:\n  name: bypass\n",
            "spec:\n",
            "  nodes:\n    - name: main\n      target: linux-aarch64\n",
            "  components:\n",
            "    - name: planner\n      node: main\n      executionClass: interactive\n      provides: [cmd.v1]\n",
            "    - name: thruster\n      node: main\n      executionClass: hard-real-time\n      role: actuator\n      requires: [cmd.v1]\n",
            "  connections:\n",
            "    - from: planner\n      to: thruster\n      contract: cmd.v1\n",
        ),
    )
    .unwrap();

    let (stdout, code) = run(&["-o", "json", "graph", "validate", path.to_str().unwrap()]);
    assert_eq!(
        code, 10,
        "invalid deployment must exit with the deployment-validation code"
    );
    ParsedEnvelope::parse(&stdout).unwrap().assert_failure();

    let _ = std::fs::remove_file(&path);
}

#[test]
fn missing_file_reports_an_error() {
    let (stdout, code) = run(&[
        "-o",
        "json",
        "graph",
        "validate",
        "/no/such/deployment.yaml",
    ]);
    // A read failure is a general failure, not a validation result.
    assert_eq!(code, 1);
    ParsedEnvelope::parse(&stdout).unwrap().assert_failure();
}
