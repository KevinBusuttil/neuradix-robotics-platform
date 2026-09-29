//! End-to-end `neuradix record migrate` against the authentic c8aa467 fixture
//! and its independently written expectations.
use std::path::{Path, PathBuf};
use std::process::Command;

use neuradix_record::{
    NativeRecordWriter, NativeRecording, RawRecord, RecordingManifest, replay_digest,
};
use neuradix_testkit::cli_output::ParsedEnvelope;
use neuradix_time::{ClockDomain, Timestamp};
use serde_json::Value;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../record/tests/fixtures/legacy-scalar-c8aa467")
}
fn fixture_bytes() -> Vec<u8> {
    let hex: String = std::fs::read_to_string(fixture_dir().join("recording.nrec.hex"))
        .unwrap()
        .split_whitespace()
        .collect();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
fn expected() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture_dir().join("expected.json")).unwrap())
        .unwrap()
}

/// A fresh scratch directory holding `in.nrec` and `provenance.json`.
struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "neuradix-cli-migrate-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("in.nrec"), fixture_bytes()).unwrap();
        std::fs::copy(
            fixture_dir().join("provenance.json"),
            dir.join("provenance.json"),
        )
        .unwrap();
        Self(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn entries(&self) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
fn migrate(s: &Scratch, input: &Path, out: &Path, extra: &[&str]) -> (String, i32) {
    let mut args = vec![
        "-o",
        "json",
        "record",
        "migrate",
        input.to_str().unwrap(),
        "--provenance",
        s.0.join("provenance.json")
            .to_str()
            .unwrap()
            .to_owned()
            .leak(),
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    run(&args)
}
fn errors(stdout: &str) -> String {
    let v: Value = serde_json::from_str(stdout).unwrap();
    assert_eq!(v["command"], "record.migrate");
    assert_eq!(v["status"], "failure");
    v["errors"].to_string()
}

#[test]
fn m1_migrates_the_historical_fixture_and_reports_after_publication() {
    let s = Scratch::new("ok");
    let (input, out) = (s.path("in.nrec"), s.path("out.nrec"));
    let (stdout, code) = migrate(&s, &input, &out, &[]);
    assert_eq!(code, 0, "{stdout}");
    let env = ParsedEnvelope::parse(&stdout).unwrap();
    env.assert_command("record.migrate").assert_success();
    let data: Value = serde_json::from_str::<Value>(&stdout).unwrap()["data"].clone();
    let exp = expected();
    assert_eq!(data["sourceDigest"], exp["source_digest"]);
    assert_eq!(data["migratedDigest"], exp["migrated_digest"]);
    assert_eq!(
        data["sourceRevision"],
        "c8aa4671bcee8739354beb7880551db7f64314fa"
    );
    assert_eq!(data["format"], "native");
    assert_eq!(data["formatVersion"], 1);
    assert_eq!(data["records"], 7);
    assert_eq!(data["opaqueChannels"], serde_json::json!([2]));
    let channels = data["channels"].as_array().unwrap();
    let summary: Vec<_> = channels
        .iter()
        .map(|c| {
            (
                c["channelId"].as_u64().unwrap(),
                c["records"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(summary, [(1, 3), (3, 2)]);
    for c in channels {
        let id = c["channelId"].as_u64().unwrap().to_string();
        assert_eq!(c["codecId"], "neuradix.scalar-le.v2");
        assert_eq!(c["wireId"], exp["channels"][&id]["canonical_wire_id"]);
        assert_eq!(c["legacyWireLen"], exp["channels"][&id]["legacy_wire_len"]);
    }
    assert_eq!(data["limits"]["maxInputBytes"], 256u64 << 20);

    // Published output is readable and matches the report.
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(data["bytes"], bytes.len());
    let migrated = NativeRecording::from_bytes(&bytes).unwrap();
    assert_eq!(
        replay_digest(&migrated),
        data["migratedDigest"].as_str().unwrap()
    );
    let source = NativeRecording::from_bytes(&fixture_bytes()).unwrap();
    // Order, channels, sequences, signed timestamps and domains are preserved;
    // the opaque channel (including its -10 ns simulation record) is untouched.
    assert_eq!(migrated.records().len(), source.records().len());
    for (m, o) in migrated.records().iter().zip(source.records()) {
        assert_eq!(
            (m.channel_id, m.sequence, m.timestamp),
            (o.channel_id, o.sequence, o.timestamp)
        );
        if m.channel_id == 2 {
            assert_eq!(m.payload, o.payload);
        }
    }
    assert!(
        migrated
            .records()
            .iter()
            .any(|r| r.channel_id == 2
                && r.timestamp == Timestamp::new(ClockDomain::Simulation, -10))
    );
    let m = migrated.manifest();
    assert!(m.channel(2).unwrap().wire.is_none());
    assert!(m.channel(1).unwrap().wire.is_some());
    assert_eq!(
        (m.format_version, &m.writer, m.seed, &m.note),
        (
            source.manifest().format_version,
            &source.manifest().writer,
            source.manifest().seed,
            &source.manifest().note
        )
    );
    // The source is unchanged and no temporary file remains.
    assert_eq!(std::fs::read(&input).unwrap(), fixture_bytes());
    assert_eq!(s.entries(), ["in.nrec", "out.nrec", "provenance.json"]);

    // Text output uses the same envelope.
    let (text, code) = run(&[
        "record",
        "migrate",
        input.to_str().unwrap(),
        "--provenance",
        s.path("provenance.json").to_str().unwrap(),
        "--out",
        s.path("out2.nrec").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("command: record.migrate"));
    assert!(text.contains("status:  success"));
    assert_eq!(std::fs::read(s.path("out2.nrec")).unwrap(), bytes);
}

fn edit_provenance(s: &Scratch, edit: impl FnOnce(&mut Value)) {
    let path = s.path("provenance.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    edit(&mut v);
    std::fs::write(path, v.to_string()).unwrap();
}

#[test]
fn m2_invalid_provenance_is_a_compatibility_failure_with_no_output() {
    type Edit = fn(&mut Value);
    let cases: [(&str, Edit, &str); 5] = [
        (
            "rev",
            |v| v["producer"]["revision"] = "c8aa467".into(),
            "revision",
        ),
        (
            "cpp",
            |v| v["producer"]["generator"] = "cpp".into(),
            "generator",
        ),
        (
            "order",
            |v| {
                let order = v["channels"][1]["field_order"].as_array_mut().unwrap();
                order.swap(0, 1);
            },
            "field order",
        ),
        (
            "unknown",
            |v| v["producer"]["target"] = "avr".into(),
            "invalid provenance",
        ),
        (
            "channel",
            |v| v["channels"][0]["channel_id"] = 9.into(),
            "no channel 9",
        ),
    ];
    for (label, edit, needle) in cases {
        let s = Scratch::new(label);
        edit_provenance(&s, edit);
        let out = s.path("out.nrec");
        let (stdout, code) = migrate(&s, &s.path("in.nrec"), &out, &[]);
        assert_eq!(code, 4, "{label}: {stdout}");
        assert!(errors(&stdout).contains(needle), "{label}: {stdout}");
        assert_eq!(s.entries(), ["in.nrec", "provenance.json"], "{label}");
    }
    let s = Scratch::new("json");
    std::fs::write(s.path("provenance.json"), b"{not json").unwrap();
    let (stdout, code) = migrate(&s, &s.path("in.nrec"), &s.path("out.nrec"), &[]);
    assert_eq!(code, 4, "{stdout}");
    let s = Scratch::new("missing");
    std::fs::remove_file(s.path("provenance.json")).unwrap();
    let (stdout, code) = migrate(&s, &s.path("in.nrec"), &s.path("out.nrec"), &[]);
    assert_eq!(code, 1, "{stdout}");
    assert_eq!(s.entries(), ["in.nrec"]);
}

fn rewrite(manifest: &RecordingManifest, records: &[RawRecord]) -> Vec<u8> {
    let mut w = NativeRecordWriter::new(Vec::new(), manifest).unwrap();
    for r in records {
        w.write_record(r.channel_id, r.sequence, r.timestamp, &r.payload)
            .unwrap();
    }
    w.finish().unwrap()
}

#[test]
fn m3_malformed_payloads_and_already_bound_channels_are_refused() {
    let source = NativeRecording::from_bytes(&fixture_bytes()).unwrap();
    let index = source
        .records()
        .iter()
        .position(|r| r.channel_id == 1)
        .unwrap();
    for (label, byte_edit) in [("trunc", 0usize), ("bool", 1)] {
        let s = Scratch::new(label);
        let mut records = source.records().to_vec();
        match byte_edit {
            0 => {
                records[index].payload.pop();
            }
            _ => records[index].payload[20] = 2,
        }
        std::fs::write(s.path("in.nrec"), rewrite(source.manifest(), &records)).unwrap();
        let (stdout, code) = migrate(&s, &s.path("in.nrec"), &s.path("out.nrec"), &[]);
        assert_eq!(code, 4, "{label}: {stdout}");
        assert!(errors(&stdout).contains("sequence 10"), "{stdout}");
        assert_eq!(s.entries(), ["in.nrec", "provenance.json"]);
    }
    // Migrating an already-migrated recording is refused (wire metadata present).
    let s = Scratch::new("bound");
    let (stdout, code) = migrate(&s, &s.path("in.nrec"), &s.path("once.nrec"), &[]);
    assert_eq!(code, 0, "{stdout}");
    let (stdout, code) = migrate(&s, &s.path("once.nrec"), &s.path("twice.nrec"), &[]);
    assert_eq!(code, 4, "{stdout}");
    assert!(errors(&stdout).contains("already records wire metadata"));
    assert!(!s.path("twice.nrec").exists());
}

#[test]
fn m4_resource_limits_are_enforced_while_reading() {
    let s = Scratch::new("limits");
    let len = fixture_bytes().len().to_string();
    let small = (fixture_bytes().len() - 1).to_string();
    let out = s.path("out.nrec");
    let (stdout, code) = migrate(&s, &s.path("in.nrec"), &out, &["--max-input-bytes", &small]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        errors(&stdout).contains("input bytes limit exceeded"),
        "{stdout}"
    );
    let (stdout, code) = migrate(&s, &s.path("in.nrec"), &out, &["--max-records", "6"]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        errors(&stdout).contains("records limit exceeded"),
        "{stdout}"
    );
    assert_eq!(s.entries(), ["in.nrec", "provenance.json"]);
    // Exactly at the limits succeeds.
    let (stdout, code) = migrate(
        &s,
        &s.path("in.nrec"),
        &out,
        &["--max-input-bytes", &len, "--max-records", "7"],
    );
    assert_eq!(code, 0, "{stdout}");
    std::fs::remove_file(&out).unwrap();
    // Limits can be lowered, never raised or zeroed (clap invalid use).
    for bad in [
        ["--max-input-bytes", "0"],
        ["--max-input-bytes", "268435457"],
        ["--max-records", "1048577"],
    ] {
        let (_, code) = migrate(&s, &s.path("in.nrec"), &out, &bad);
        assert_eq!(code, 2, "{bad:?}");
    }
    // An oversized provenance document is rejected while reading.
    let mut big = b"{\"pad\":\"".to_vec();
    big.resize(1 << 20, b'x');
    big.extend_from_slice(b"\"}");
    std::fs::write(s.path("provenance.json"), big).unwrap();
    let (stdout, code) = migrate(&s, &s.path("in.nrec"), &out, &[]);
    assert_eq!(code, 1, "{stdout}");
    assert!(
        errors(&stdout).contains("exceeds 1048576 bytes"),
        "{stdout}"
    );
    assert!(!out.exists());
}

#[test]
fn m5_existing_destinations_and_in_place_requests_are_refused() {
    let s = Scratch::new("dest");
    let input = s.path("in.nrec");
    let (stdout, code) = migrate(&s, &input, &input, &[]);
    assert_eq!(code, 2, "{stdout}");
    assert!(errors(&stdout).contains("in-place"), "{stdout}");
    // A hard link to the input is the input.
    std::fs::hard_link(&input, s.path("alias.nrec")).unwrap();
    let (stdout, code) = migrate(&s, &input, &s.path("alias.nrec"), &[]);
    assert_eq!(code, 2, "{stdout}");
    assert!(errors(&stdout).contains("in-place"), "{stdout}");
    // A symlink to the input, an existing file and a dangling symlink.
    std::os::unix::fs::symlink(&input, s.path("link.nrec")).unwrap();
    let (stdout, code) = migrate(&s, &input, &s.path("link.nrec"), &[]);
    assert_eq!(code, 2, "{stdout}");
    assert!(errors(&stdout).contains("in-place"), "{stdout}");
    std::fs::write(s.path("taken.nrec"), b"keep").unwrap();
    let (stdout, code) = migrate(&s, &input, &s.path("taken.nrec"), &[]);
    assert_eq!(code, 2, "{stdout}");
    assert!(
        errors(&stdout).contains("refusing to overwrite"),
        "{stdout}"
    );
    assert_eq!(std::fs::read(s.path("taken.nrec")).unwrap(), b"keep");
    std::os::unix::fs::symlink(s.path("nowhere"), s.path("dangling.nrec")).unwrap();
    let (stdout, code) = migrate(&s, &input, &s.path("dangling.nrec"), &[]);
    assert_eq!(code, 2, "{stdout}");
    assert!(!s.path("nowhere").exists());
    // The provenance file is an existing destination too.
    let (_, code) = migrate(&s, &input, &s.path("provenance.json"), &[]);
    assert_eq!(code, 2);
    assert_eq!(std::fs::read(&input).unwrap(), fixture_bytes());
    assert_eq!(
        s.entries(),
        [
            "alias.nrec",
            "dangling.nrec",
            "in.nrec",
            "link.nrec",
            "provenance.json",
            "taken.nrec"
        ]
    );
}

#[test]
fn m6_io_failures_and_foreign_inputs_leave_nothing_behind() {
    let s = Scratch::new("io");
    let (stdout, code) = migrate(&s, &s.path("in.nrec"), &s.path("absent/out.nrec"), &[]);
    assert_eq!(code, 1, "{stdout}");
    assert!(errors(&stdout).contains("does not exist"), "{stdout}");
    let (stdout, code) = migrate(&s, &s.path("missing.nrec"), &s.path("out.nrec"), &[]);
    assert_eq!(code, 1, "{stdout}");
    // MCAP or other containers are not migrated (native to native only).
    let mut mcap = neuradix_record::MCAP_MAGIC.to_vec();
    mcap.extend_from_slice(&[0; 32]);
    std::fs::write(s.path("in.mcap"), mcap).unwrap();
    let (stdout, code) = migrate(&s, &s.path("in.mcap"), &s.path("out.nrec"), &[]);
    assert_eq!(code, 1, "{stdout}");
    assert!(errors(&stdout).contains("native (.nrec) recordings only"));
    // A truncated native container fails decoding.
    let bytes = fixture_bytes();
    std::fs::write(s.path("cut.nrec"), &bytes[..bytes.len() - 3]).unwrap();
    let (stdout, code) = migrate(&s, &s.path("cut.nrec"), &s.path("out.nrec"), &[]);
    assert_eq!(code, 1, "{stdout}");
    assert_eq!(
        s.entries(),
        ["cut.nrec", "in.mcap", "in.nrec", "provenance.json"]
    );
}
