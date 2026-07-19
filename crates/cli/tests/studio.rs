//! End-to-end tests for `neuradix studio timeline` and `neuradix studio series`.

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

/// Write a small recording with a depth channel and a command-lineage channel.
fn write_recording(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    let manifest = RecordingManifest::builder("neuradix-cli-studio-test")
        .channel(Channel {
            id: 0,
            name: "navigation/depth".to_owned(),
            schema_id: "sha256:depth".to_owned(),
            clock_domain: "simulation".to_owned(),
        })
        .channel(Channel {
            id: 1,
            name: LINEAGE_CHANNEL.to_owned(),
            schema_id: "sha256:lineage".to_owned(),
            clock_domain: "monotonic".to_owned(),
        })
        .build();

    let mut w = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    for i in 0..4u64 {
        // Depth samples on channel 0 at 25 ms spacing (40 Hz).
        let ts = Timestamp::new(ClockDomain::Simulation, i as i128 * 25_000_000);
        w.write_record(0, i, ts, &(i as f64).to_le_bytes()).unwrap();

        // Command lineage on channel 1.
        let lineage = CommandLineage {
            trace: i,
            at_nanos: i as i128 * 25_000_000,
            clock_domain: "monotonic".to_owned(),
            origin: LineageOrigin {
                source: "navigation/depth".to_owned(),
                quantity: "depth".to_owned(),
                value: i as f64,
                unit: "m".to_owned(),
            },
            holder: "controller".to_owned(),
            capability: "propulsion/vertical-thrust".to_owned(),
            requested: i as f64 * 0.5,
            outcome: "accepted".to_owned(),
            applied: (i as f64 * 0.5).min(0.8),
            acted_rules: vec![],
            reject_reason: None,
        };
        let ts = Timestamp::new(ClockDomain::Monotonic, i as i128 * 25_000_000);
        w.write_record(1, i, ts, &lineage.to_json_bytes()).unwrap();
    }
    std::fs::write(&path, w.finish().unwrap()).unwrap();
    path
}

#[test]
fn studio_timeline_reports_channels_and_domains() {
    let path = write_recording("neuradix-cli-studio-timeline.nrec");
    let (stdout, code) = run(&["-o", "json", "studio", "timeline", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_command("studio.timeline").assert_success();
    assert_eq!(env.data_field("messages").unwrap(), 8);
    assert_eq!(env.data_field("channels").unwrap(), 2);
    // Two domains: simulation (depth) and monotonic (lineage).
    let domains = env
        .data_field("domains")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(domains.len(), 2);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn studio_series_extracts_applied_from_lineage() {
    let path = write_recording("neuradix-cli-studio-series.nrec");
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
    env.assert_command("studio.series").assert_success();
    assert_eq!(env.data_field("field").unwrap(), "applied");
    assert_eq!(env.data_field("count").unwrap(), 4);
    // The lineage channel was auto-selected.
    assert_eq!(env.data_field("channel").unwrap(), 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn studio_series_unknown_field_fails() {
    let path = write_recording("neuradix-cli-studio-badfield.nrec");
    let (stdout, code) = run(&[
        "studio",
        "series",
        path.to_str().unwrap(),
        "--field",
        "nonexistent",
    ]);
    assert_eq!(code, 1);
    assert!(stdout.contains("not found") || stdout.contains("nonexistent"));
    let _ = std::fs::remove_file(&path);
}
