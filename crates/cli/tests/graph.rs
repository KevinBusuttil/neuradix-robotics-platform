//! End-to-end tests for `neuradix graph validate`.

use std::process::Command;

use neuradix_testkit::cli_output::ParsedEnvelope;

const REFERENCE_DEPLOYMENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/reference-auv/deployment.yaml"
);

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
