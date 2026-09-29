//! End-to-end `neuradix channel manifest|verify|table` (WP-A02.4) against the
//! existing golden contracts, pinned digest and golden board table.
use std::path::{Path, PathBuf};
use std::process::Command;

use neuradix_contracts::ChannelManifest;
use serde_json::Value;

/// Pinned by crates/embedded-codegen/tests/golden/channel_table.rs.
const GOLDEN_DIGEST: &str =
    "sha256:4a8cf8f6ba9d3479fad842b80b177e0d90b93bcf259f070fd37e4ac9a00d97fb";
const DEPTH_WIRE_ID: &str =
    "sha256:4780f56bd7e4780987152907da8156c19dd2946470bc550c35abb16285d7b11d";

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
        .canonicalize()
        .unwrap()
}
fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/channels")
}
fn depth() -> PathBuf {
    repo("contracts/standard/navigation/vehicle-depth.yaml")
}
fn tiny() -> PathBuf {
    repo("crates/embedded-codegen/tests/fixtures/tiny-telemetry.yaml")
}
fn depth_reversed() -> PathBuf {
    repo("crates/record/tests/fixtures/legacy-scalar-c8aa467/depth-reversed.yaml")
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "neuradix-cli-channel-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
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
    /// Write a binding specification with the given (id, name, contract) rows.
    fn spec(&self, name: &str, rows: &[(u64, &str, &Path)]) -> PathBuf {
        let mut text = String::from(
            "apiVersion: channels.neuradix.io/v1alpha1\nkind: ChannelBindings\nchannels:\n",
        );
        for (id, channel, contract) in rows {
            text.push_str(&format!(
                "  - compactId: {id}\n    name: {channel}\n    contract: {}\n",
                contract.display()
            ));
        }
        let path = self.path(name);
        std::fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> (Value, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_neuradix"))
        .arg("-o")
        .arg("json")
        .args(args)
        .output()
        .expect("neuradix binary should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    (
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}")),
        output.status.code().unwrap_or(-1),
    )
}
fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}
fn manifest(spec: &Path, out: &Path) -> (Value, i32) {
    run(&["channel", "manifest", s(spec), "--out", s(out)])
}
fn verify(m: &Path, bindings: Option<&Path>) -> (Value, i32) {
    match bindings {
        Some(b) => run(&["channel", "verify", s(m), "--bindings", s(b)]),
        None => run(&["channel", "verify", s(m)]),
    }
}
fn errors(v: &Value) -> String {
    assert_eq!(v["status"], "failure", "{v}");
    v["errors"].to_string()
}

#[test]
fn c1_generation_is_deterministic_and_matches_pinned_digest() {
    let sc = Scratch::new("gen");
    let golden = fixtures().join("bindings.yaml");
    let (v, code) = manifest(&golden, &sc.path("m.json"));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "channel.manifest");
    assert_eq!(v["data"]["digest"], GOLDEN_DIGEST);
    assert_eq!(v["data"]["channels"][0]["wireId"], DEPTH_WIRE_ID);
    assert_eq!(v["data"]["manifestVersion"], "neuradix.channel-manifest.v2");
    let bytes = std::fs::read_to_string(sc.path("m.json")).unwrap();
    assert_eq!(v["data"]["bytes"], bytes.len());
    // Round trip through the library parser and serializer.
    let parsed = ChannelManifest::parse(&bytes).unwrap();
    assert_eq!(parsed.digest(), GOLDEN_DIGEST);
    assert_eq!(parsed.to_json_pretty(), bytes);

    // Reordered entries and reordered (equivalent) contract fields give
    // byte-identical output.
    let reordered = sc.spec(
        "reordered.yaml",
        &[
            (2, "tiny-telemetry", &tiny()),
            (1, "vehicle-depth", &depth_reversed()),
        ],
    );
    let (v, code) = manifest(&reordered, &sc.path("m2.json"));
    assert_eq!(code, 0, "{v}");
    assert_eq!(std::fs::read_to_string(sc.path("m2.json")).unwrap(), bytes);

    // One contract on several distinctly named channels keeps explicit IDs.
    let shared = sc.spec(
        "shared.yaml",
        &[(1, "depth-a", &depth()), (9, "depth-b", &depth())],
    );
    let (v, code) = manifest(&shared, &sc.path("m3.json"));
    assert_eq!(code, 0, "{v}");
    let ids: Vec<_> = v["data"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["compactId"].as_u64().unwrap(), c["wireId"].clone()))
        .collect();
    assert_eq!(ids, [(1, DEPTH_WIRE_ID.into()), (9, DEPTH_WIRE_ID.into())]);
    assert_eq!(
        v["data"]["contractBytes"],
        std::fs::metadata(depth()).unwrap().len()
    );
}

#[test]
fn c2_verification_levels_are_distinct() {
    let sc = Scratch::new("levels");
    let bindings = fixtures().join("bindings.yaml");
    assert_eq!(manifest(&bindings, &sc.path("m.json")).1, 0);
    let (v, code) = verify(&sc.path("m.json"), None);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "self-consistent");
    assert_eq!(v["data"]["matchesBindings"], Value::Null);
    assert!(v["warnings"].to_string().contains("self-consistent only"));
    let (v, code) = verify(&sc.path("m.json"), Some(&bindings));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "matches-bindings");
    assert_eq!(v["data"]["matchesBindings"], true);
    assert_eq!(v["data"]["expectedDigest"], GOLDEN_DIGEST);
}

#[test]
fn c3_self_consistent_but_wrong_manifests_fail_exact_verification() {
    let sc = Scratch::new("wrong");
    let bindings = fixtures().join("bindings.yaml");
    type Rows<'a> = Vec<(u64, &'a str, PathBuf)>;
    let cases: [(&str, Rows, &str); 4] = [
        // Equal-width foreign layout on channel 1, with a correct digest.
        (
            "foreign",
            vec![
                (1, "vehicle-depth", fixtures().join("range.yaml")),
                (2, "tiny-telemetry", tiny()),
            ],
            "channel 1: channel `vehicle-depth` does not carry the expected wire layout",
        ),
        (
            "missing",
            vec![(1, "vehicle-depth", depth())],
            "missing channel 2",
        ),
        (
            "extra",
            vec![
                (1, "vehicle-depth", depth()),
                (2, "tiny-telemetry", tiny()),
                (3, "spare", depth()),
            ],
            "unexpected channel 3",
        ),
        (
            "renamed",
            vec![(1, "depth", depth()), (2, "tiny-telemetry", tiny())],
            "name `depth` differs",
        ),
    ];
    for (label, rows, needle) in cases {
        let rows: Vec<(u64, &str, &Path)> =
            rows.iter().map(|(i, n, p)| (*i, *n, p.as_path())).collect();
        let spec = sc.spec(&format!("{label}.yaml"), &rows);
        let out = sc.path(&format!("{label}.json"));
        assert_eq!(manifest(&spec, &out).1, 0, "{label}");
        // The manifest is valid on its own...
        let (v, code) = verify(&out, None);
        assert_eq!(
            (code, v["data"]["level"].clone()),
            (0, "self-consistent".into())
        );
        // ...but does not match the expected bindings.
        let (v, code) = verify(&out, Some(&bindings));
        assert_eq!(code, 4, "{label}: {v}");
        assert_eq!(v["data"]["matchesBindings"], false);
        assert!(errors(&v).contains(needle), "{label}: {v}");
    }
}

#[test]
fn c4_tampered_or_unknown_manifests_are_not_self_consistent() {
    let sc = Scratch::new("tamper");
    assert_eq!(
        manifest(&fixtures().join("bindings.yaml"), &sc.path("m.json")).1,
        0
    );
    let good = std::fs::read_to_string(sc.path("m.json")).unwrap();
    type Edit = Box<dyn Fn(&mut Value)>;
    let edits: [(&str, Edit, &str); 6] = [
        (
            "digest",
            Box::new(|v| v["digest"] = format!("sha256:{}", "0".repeat(64)).into()),
            "digest mismatch",
        ),
        (
            "version",
            Box::new(|v| v["manifest_version"] = "neuradix.channel-manifest.v1".into()),
            "unsupported channel manifest version",
        ),
        (
            "field",
            Box::new(|v| v["channels"][0]["session"] = 1.into()),
            "unknown field",
        ),
        (
            "identity",
            Box::new(|v| v["channels"][0]["wire_id"] = DEPTH_WIRE_ID.to_uppercase().into()),
            "malformed",
        ),
        (
            "reserved",
            Box::new(|v| v["channels"][0]["compact_id"] = 0.into()),
            "reserved compact ID 0",
        ),
        (
            "collision",
            Box::new(|v| v["channels"][1]["compact_id"] = 1.into()),
            "bound to both",
        ),
    ];
    for (label, edit, needle) in edits {
        let mut v: Value = serde_json::from_str(&good).unwrap();
        edit(&mut v);
        let path = sc.path(&format!("{label}.json"));
        std::fs::write(&path, v.to_string()).unwrap();
        let (r, code) = verify(&path, None);
        assert_eq!(code, 4, "{label}: {r}");
        assert!(errors(&r).contains(needle), "{label}: {r}");
    }
    std::fs::write(sc.path("junk.json"), "{").unwrap();
    assert_eq!(verify(&sc.path("junk.json"), None).1, 4);
    assert_eq!(verify(&sc.path("absent.json"), None).1, 1);
    let big = format!("{{\"x\":\"{}\"}}", "y".repeat(1 << 20));
    std::fs::write(sc.path("big.json"), big).unwrap();
    let (r, code) = verify(&sc.path("big.json"), None);
    assert_eq!(code, 1, "{r}");
    assert!(errors(&r).contains("exceeds 1048576 bytes"));
}

#[test]
fn c5_invalid_specifications_and_contracts_are_rejected_before_output() {
    let sc = Scratch::new("spec");
    let out = sc.path("m.json");
    let d = depth();
    let cases: [(&str, String, i32, &str); 8] = [
        (
            "dup-id",
            spec_text(&[(1, "a", &d), (1, "b", &d)]),
            4,
            "bound to both",
        ),
        (
            "reserved",
            spec_text(&[(0, "a", &d)]),
            4,
            "reserved compact ID 0",
        ),
        (
            "dup-name",
            spec_text(&[(1, "a", &d), (2, "a", &d)]),
            4,
            "declared more than once",
        ),
        (
            "identity-field",
            spec_text(&[(1, "a", &d)]) + "    wireId: sha256:00\n",
            4,
            "unknown field",
        ),
        (
            "version",
            spec_text(&[(1, "a", &d)]).replace("v1alpha1", "v9"),
            4,
            "expected apiVersion",
        ),
        (
            "unsupported",
            spec_text(&[(1, "a", &fixtures().join("labelled.yaml"))]),
            3,
            "unsupported scalar type",
        ),
        (
            "missing-contract",
            spec_text(&[(1, "a", &sc.path("absent.yaml"))]),
            1,
            "could not read contract",
        ),
        (
            "too-many",
            spec_text(
                &(1..=257u64)
                    .map(|i| (i, "c", d.as_path()))
                    .collect::<Vec<_>>(),
            ),
            4,
            "exceed the limit of 256",
        ),
    ];
    for (label, text, exit, needle) in cases {
        let spec = sc.path(&format!("{label}.yaml"));
        std::fs::write(&spec, text).unwrap();
        let (v, code) = manifest(&spec, &out);
        assert_eq!(code, exit, "{label}: {v}");
        assert!(errors(&v).contains(needle), "{label}: {v}");
        assert!(!out.exists(), "{label}");
    }
    // Invalid contract YAML is a contract-validation failure.
    std::fs::write(sc.path("bad.yaml"), "apiVersion: nope\n").unwrap();
    let spec = sc.spec("bad-contract.yaml", &[(1, "a", &sc.path("bad.yaml"))]);
    assert_eq!(manifest(&spec, &out).1, 3);
    // Byte limits apply while reading the specification and each contract.
    std::fs::write(sc.path("huge-spec.yaml"), "#".repeat(64 << 10) + "\n").unwrap();
    let (v, code) = manifest(&sc.path("huge-spec.yaml"), &out);
    assert_eq!(code, 1, "{v}");
    assert!(errors(&v).contains("exceeds 65536 bytes"));
    let padded = "#".repeat(64 << 10) + "\n" + &std::fs::read_to_string(&d).unwrap();
    std::fs::write(sc.path("padded.yaml"), padded).unwrap();
    let spec = sc.spec("padded-spec.yaml", &[(1, "a", &sc.path("padded.yaml"))]);
    let (v, code) = manifest(&spec, &out);
    assert_eq!(code, 1, "{v}");
    assert!(errors(&v).contains("exceeds 65536 bytes"));
    assert_eq!(manifest(&sc.path("absent-spec.yaml"), &out).1, 1);
    // The total over distinct contracts is bounded too: 65 files of ~64 KiB.
    let body = std::fs::read_to_string(&d).unwrap();
    let pad = "#".repeat((64 << 10) - body.len() - 1) + "\n";
    let rows: Vec<PathBuf> = (1..=65u64)
        .map(|i| {
            let path = sc.path(&format!("c{i}.yaml"));
            std::fs::write(&path, format!("{pad}{body}")).unwrap();
            path
        })
        .collect();
    let named: Vec<String> = (1..=65).map(|i| format!("c{i}")).collect();
    let rows: Vec<(u64, &str, &Path)> = rows
        .iter()
        .zip(&named)
        .enumerate()
        .map(|(i, (p, n))| (i as u64 + 1, n.as_str(), p.as_path()))
        .collect();
    let spec = sc.spec("total.yaml", &rows);
    let (v, code) = manifest(&spec, &out);
    assert_eq!(code, 1, "{v}");
    assert!(errors(&v).contains("4194304 bytes in total"), "{v}");
    assert!(!out.exists());
}

fn spec_text(rows: &[(u64, &str, &Path)]) -> String {
    let mut text = String::from(
        "apiVersion: channels.neuradix.io/v1alpha1\nkind: ChannelBindings\nchannels:\n",
    );
    for (id, name, contract) in rows {
        text.push_str(&format!(
            "  - compactId: {id}\n    name: {name}\n    contract: {}\n",
            contract.display()
        ));
    }
    text
}

#[test]
fn c6_board_table_is_the_golden_generated_source() {
    let sc = Scratch::new("table");
    let (v, code) = run(&[
        "channel",
        "table",
        s(&fixtures().join("bindings.yaml")),
        "--out",
        s(&sc.path("table.rs")),
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "channel.table");
    assert_eq!(v["data"]["digest"], GOLDEN_DIGEST);
    // Byte-identical to the golden that channel_binding.rs E7 compiles, builds
    // into a ChannelTable, and rejects after editing a binding or the digest.
    assert_eq!(
        std::fs::read_to_string(sc.path("table.rs")).unwrap(),
        std::fs::read_to_string(repo(
            "crates/embedded-codegen/tests/golden/channel_table.rs"
        ))
        .unwrap()
    );
    assert_eq!(sc.entries(), ["table.rs"]);
}

#[test]
fn c7_outputs_never_replace_existing_files_or_inputs() {
    let sc = Scratch::new("dest");
    let spec = sc.spec("b.yaml", &[(1, "vehicle-depth", &depth())]);
    std::fs::write(sc.path("taken.json"), b"keep").unwrap();
    for (op, out) in [
        ("manifest", sc.path("taken.json")),
        ("table", sc.path("taken.json")),
        ("manifest", spec.clone()),
    ] {
        let (v, code) = run(&["channel", op, s(&spec), "--out", s(&out)]);
        assert_eq!(code, 2, "{op}: {v}");
        assert!(errors(&v).contains("refusing to overwrite"), "{v}");
    }
    assert_eq!(std::fs::read(sc.path("taken.json")).unwrap(), b"keep");
    // A contract read by the command is an input too (here via a symlink).
    std::os::unix::fs::symlink(depth(), sc.path("contract-link.yaml")).unwrap();
    let (v, code) = manifest(&spec, &sc.path("contract-link.yaml"));
    assert_eq!(code, 2, "{v}");
    let (v, code) = manifest(&spec, &sc.path("absent/m.json"));
    assert_eq!(code, 1, "{v}");
    assert!(errors(&v).contains("does not exist"));
    assert_eq!(sc.entries(), ["b.yaml", "contract-link.yaml", "taken.json"]);
    // Text output uses the same envelope.
    let out = Command::new(env!("CARGO_BIN_EXE_neuradix"))
        .args([
            "channel",
            "manifest",
            s(&spec),
            "--out",
            s(&sc.path("t.json")),
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("command: channel.manifest"));
    assert!(text.contains("status:  success"));
}
