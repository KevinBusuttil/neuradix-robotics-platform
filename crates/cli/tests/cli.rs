//! End-to-end tests for the `neuradix` binary, covering the output envelope and
//! exit-code contract.

use std::process::Command;

use neuradix_testkit::cli_output::ParsedEnvelope;

const REFERENCE_CONTRACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/standard/navigation/vehicle-depth.yaml"
);

const EXPECTED_SCHEMA_ID: &str =
    "sha256:4c9c5d9381658f7779ef0d3ef11eda3f29006f7e751d06dba40a12d6f4ce2a73";

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
fn version_produces_a_wellformed_json_envelope() {
    let (stdout, code) = run(&["--output", "json", "version"]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).expect("valid json envelope");
    env.assert_well_formed()
        .assert_command("version")
        .assert_success();
    assert_eq!(env.data_field("name").unwrap(), "neuradix");
}

#[test]
fn validate_reference_contract_succeeds() {
    let (stdout, code) = run(&["-o", "json", "contract", "validate", REFERENCE_CONTRACT]);
    assert_eq!(code, 0);
    ParsedEnvelope::parse(&stdout)
        .unwrap()
        .assert_command("contract.validate")
        .assert_success();
}

#[test]
fn hash_reports_the_expected_schema_identity() {
    let (stdout, code) = run(&["-o", "json", "contract", "hash", REFERENCE_CONTRACT]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    assert_eq!(env.data_field("schemaId").unwrap(), EXPECTED_SCHEMA_ID);
}

#[test]
fn invalid_contract_exits_with_code_3() {
    let dir = std::env::temp_dir().join("neuradix-cli-it");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("invalid.yaml");
    std::fs::write(&path, "apiVersion: wrong/v1\nkind: StreamContract\n").unwrap();

    let (stdout, code) = run(&["-o", "json", "contract", "validate", path.to_str().unwrap()]);
    assert_eq!(
        code, 3,
        "invalid contract must exit with the contract-validation code"
    );
    ParsedEnvelope::parse(&stdout).unwrap().assert_failure();

    let _ = std::fs::remove_file(&path);
}

#[test]
fn table_output_is_human_readable() {
    let (stdout, code) = run(&["version"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("command: version"));
    assert!(stdout.contains("status:  success"));
}

#[test]
fn yaml_output_is_emitted() {
    let (stdout, code) = run(&["--output", "yaml", "version"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("apiVersion: cli.neuradix.io/v1alpha1"));
    assert!(stdout.contains("command: version"));
}
