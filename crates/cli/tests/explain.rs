//! End-to-end tests for `neuradix explain command`.

use std::process::Command;

use neuradix_record::{Channel, NativeRecordWriter, RecordingManifest};
use neuradix_safety::{CommandLineage, LINEAGE_CHANNEL, LineageOrigin};
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

fn entry(
    trace: u64,
    at_nanos: i128,
    outcome: &str,
    applied: f64,
    reject: Option<&str>,
) -> CommandLineage {
    CommandLineage {
        command_metadata: None,
        trace,
        at_nanos,
        clock_domain: "monotonic".to_owned(),
        origin: LineageOrigin::new("navigation/vehicle-depth", "depth", "m", 12.0),
        holder: "depth-controller".to_owned(),
        capability: "propulsion/vertical-thrust".to_owned(),
        requested: 1.0,
        outcome: outcome.to_owned(),
        applied,
        acted_rules: Vec::new(),
        reject_reason: reject.map(str::to_owned),
    }
}

/// Write a lineage recording with an accepted command at 100 and a rejected one at 200.
fn write_lineage(name: &str, with_lineage: bool) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    let channel_name = if with_lineage {
        LINEAGE_CHANNEL
    } else {
        "other/channel"
    };
    let manifest = RecordingManifest::builder("explain-test")
        .channel(Channel {
            id: 0,
            name: channel_name.to_owned(),
            schema_id: "application/vnd.neuradix.command-lineage+json".to_owned(),
            clock_domain: "monotonic".to_owned(),
        })
        .build();
    let mut writer = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    let entries = [
        entry(0, 100, "accepted", 1.0, None),
        entry(
            1,
            200,
            "rejected",
            0.0,
            Some("authority denied: authority lease has expired"),
        ),
    ];
    for e in &entries {
        writer
            .write_record(
                0,
                e.trace,
                Timestamp::new(ClockDomain::Monotonic, e.at_nanos),
                &e.to_json_bytes(),
            )
            .unwrap();
    }
    std::fs::write(&path, writer.finish().unwrap()).unwrap();
    path
}

#[test]
fn explains_the_nearest_command() {
    let path = write_lineage("neuradix-explain-ok.nrec", true);
    let (stdout, code) = run(&[
        "-o",
        "json",
        "explain",
        "command",
        path.to_str().unwrap(),
        "--at",
        "110",
    ]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_command("explain.command").assert_success();
    // Nearest to 110 is the accepted command at 100.
    let command = env.data_field("command").unwrap();
    assert_eq!(command.get("outcome").unwrap(), "accepted");
    assert_eq!(command.get("atNanos").unwrap(), 100);
    // The full four-stage chain is present.
    assert_eq!(
        env.data_field("lineage").unwrap().as_array().unwrap().len(),
        4
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn selects_the_rejected_command_when_nearer() {
    let path = write_lineage("neuradix-explain-rej.nrec", true);
    let (stdout, code) = run(&[
        "-o",
        "json",
        "explain",
        "command",
        path.to_str().unwrap(),
        "--at",
        "210",
    ]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    let command = env.data_field("command").unwrap();
    assert_eq!(command.get("outcome").unwrap(), "rejected");
    assert_eq!(command.get("atNanos").unwrap(), 200);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn recording_without_lineage_fails() {
    let path = write_lineage("neuradix-explain-none.nrec", false);
    let (_stdout, code) = run(&["explain", "command", path.to_str().unwrap(), "--at", "100"]);
    assert_eq!(code, 1, "a recording with no lineage channel must fail");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn non_finite_lineage_remains_auditable_and_cannot_poison_numeric_series() {
    let path = std::env::temp_dir().join("neuradix-explain-nonfinite.nrec");
    let manifest = RecordingManifest::builder("nonfinite-test")
        .channel(Channel {
            id: 0,
            name: LINEAGE_CHANNEL.to_owned(),
            schema_id: "lineage".to_owned(),
            clock_domain: "monotonic".to_owned(),
        })
        .build();
    let mut writer = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    for (i, value) in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY]
        .into_iter()
        .enumerate()
    {
        let mut e = entry(
            i as u64,
            i as i128,
            "rejected",
            0.0,
            Some("NonFiniteCommand"),
        );
        e.requested = value;
        writer
            .write_record(
                0,
                e.trace,
                Timestamp::new(ClockDomain::Monotonic, e.at_nanos),
                &e.to_json_bytes(),
            )
            .unwrap();
    }
    std::fs::write(&path, writer.finish().unwrap()).unwrap();
    for (at, marker) in [("0", "NaN"), ("1", "Infinity"), ("2", "-Infinity")] {
        let (stdout, code) = run(&[
            "-o",
            "json",
            "explain",
            "command",
            path.to_str().unwrap(),
            "--at",
            at,
        ]);
        assert_eq!(code, 0);
        let env = ParsedEnvelope::parse(&stdout).unwrap();
        assert_eq!(env.data_field("lineage").unwrap()[1]["requested"], marker);
    }
    let (stdout, code) = run(&[
        "-o",
        "json",
        "studio",
        "series",
        path.to_str().unwrap(),
        "--field",
        "requested",
    ]);
    assert_eq!(code, 1);
    assert!(stdout.contains("non-finite"));
    let (stdout, code) = run(&[
        "-o",
        "json",
        "studio",
        "series",
        path.to_str().unwrap(),
        "--field",
        "applied",
    ]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    assert_eq!(env.data_field("count").unwrap(), 3);
    std::fs::remove_file(path).unwrap();
}
