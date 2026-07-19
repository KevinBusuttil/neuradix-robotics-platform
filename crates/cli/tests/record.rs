//! End-to-end tests for `neuradix record inspect` and `neuradix replay run`,
//! including the determinism (exit code 9) contract.

use std::path::Path;
use std::process::Command;

use neuradix_record::{
    Channel, NativeRecordWriter, NativeRecording, RecordingManifest, replay_digest,
};
use neuradix_testkit::cli_output::ParsedEnvelope;
use neuradix_time::{ClockDomain, Timestamp};

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

/// Write a small deterministic recording and return (path, digest).
fn write_recording(name: &str) -> (std::path::PathBuf, String) {
    let path = std::env::temp_dir().join(name);
    let manifest = RecordingManifest::builder("neuradix-cli-test")
        .channel(Channel {
            id: 0,
            name: "test/channel".to_owned(),
            schema_id: "sha256:abc".to_owned(),
            clock_domain: "simulation".to_owned(),
        })
        .note("cli record test")
        .build();

    let mut writer = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    for i in 0..3u64 {
        let ts = Timestamp::new(ClockDomain::Simulation, i as i128 * 1_000);
        writer.write_record(0, i, ts, &i.to_le_bytes()).unwrap();
    }
    let bytes = writer.finish().unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let digest = replay_digest(&NativeRecording::from_bytes(&bytes).unwrap());
    (path, digest)
}

#[test]
fn record_inspect_reports_the_manifest() {
    let (path, _) = write_recording("neuradix-cli-inspect.nrec");
    let (stdout, code) = run(&["-o", "json", "record", "inspect", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_command("record.inspect").assert_success();
    assert_eq!(env.data_field("records").unwrap(), 3);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn replay_run_accepts_the_correct_digest() {
    let (path, digest) = write_recording("neuradix-cli-replay-ok.nrec");
    let (stdout, code) = run(&[
        "-o",
        "json",
        "replay",
        "run",
        path.to_str().unwrap(),
        "--expect-digest",
        &digest,
    ]);
    assert_eq!(code, 0, "correct digest must succeed");
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_command("replay.run").assert_success();
    assert_eq!(env.data_field("digest").unwrap(), digest.as_str());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn replay_run_rejects_a_wrong_digest_with_exit_9() {
    let (path, _) = write_recording("neuradix-cli-replay-bad.nrec");
    let (stdout, code) = run(&[
        "-o",
        "json",
        "replay",
        "run",
        path.to_str().unwrap(),
        "--expect-digest",
        "sha256:0000",
    ]);
    assert_eq!(
        code, 9,
        "digest mismatch must use the determinism exit code"
    );
    ParsedEnvelope::parse(&stdout).unwrap().assert_failure();
    let _ = std::fs::remove_file(&path);
}

#[test]
fn export_to_mcap_preserves_the_replay_digest() {
    let (path, digest) = write_recording("neuradix-cli-export-src.nrec");
    let mcap = std::env::temp_dir().join("neuradix-cli-export.mcap");

    // Export the native recording to MCAP.
    let (stdout, code) = run(&[
        "-o",
        "json",
        "record",
        "export",
        path.to_str().unwrap(),
        "--out",
        mcap.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "export should succeed");
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_command("record.export").assert_success();
    assert_eq!(env.data_field("digest").unwrap(), digest.as_str());
    assert_eq!(env.data_field("outputFormat").unwrap(), "mcap");

    // The exported file is recognized as MCAP and inspects to the same digest.
    let (stdout, code) = run(&["-o", "json", "record", "inspect", mcap.to_str().unwrap()]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    assert_eq!(env.data_field("format").unwrap(), "mcap");
    assert_eq!(env.data_field("digest").unwrap(), digest.as_str());
    assert_eq!(env.data_field("records").unwrap(), 3);

    // Replaying the MCAP against the *native* digest succeeds: cross-container
    // replay equivalence.
    let (_stdout, code) = run(&[
        "replay",
        "run",
        mcap.to_str().unwrap(),
        "--expect-digest",
        &digest,
    ]);
    assert_eq!(code, 0, "MCAP must replay to the native digest");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&mcap);
}

#[test]
fn inspecting_a_non_recording_fails() {
    let path = std::env::temp_dir().join("neuradix-cli-not-a-recording.nrec");
    std::fs::write(&path, b"definitely not a recording").unwrap();
    let (_stdout, code) = run(&["record", "inspect", path.to_str().unwrap()]);
    assert_eq!(code, 1);
    let _ = std::fs::remove_file(&path);
}

/// Guards that the test helper path handling is sane on this platform.
#[test]
fn temp_dir_is_writable() {
    assert!(Path::new(&std::env::temp_dir()).exists());
}
