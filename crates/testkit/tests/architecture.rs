//! Enforces the workspace dependency-direction rules from the implementation
//! plan by inspecting each crate's `[dependencies]` section. Cargo already
//! prevents dependency *cycles*; this test additionally prevents forbidden
//! *directions* (for example, `contracts` must depend on no internal crate, and
//! nothing may depend on `cli` or `testkit` as a normal dependency).

use std::collections::BTreeSet;
use std::path::PathBuf;

/// The internal (non-dev) dependencies each crate is permitted to have.
fn allowed(crate_name: &str) -> BTreeSet<&'static str> {
    match crate_name {
        "neuradix-contracts" => BTreeSet::new(),
        "neuradix-time" => BTreeSet::new(),
        "neuradix-transport-api" => ["neuradix-contracts"].into_iter().collect(),
        "neuradix-runtime" => ["neuradix-contracts", "neuradix-time"]
            .into_iter()
            .collect(),
        "neuradix-record" => ["neuradix-contracts", "neuradix-time"]
            .into_iter()
            .collect(),
        "neuradix-safety" => ["neuradix-time", "neuradix-runtime"].into_iter().collect(),
        "neuradix-python" => ["neuradix-runtime"].into_iter().collect(),
        "neuradix-graph" => ["neuradix-contracts"].into_iter().collect(),
        "neuradix-sim" => ["neuradix-time"].into_iter().collect(),
        "neuradix-studio" => ["neuradix-record", "neuradix-time"].into_iter().collect(),
        "neuradix-embedded-core" => ["neuradix-time"].into_iter().collect(),
        // Framing depends only on `core`; its embedded-core/time deps are
        // dev-only (integration tests), which this check does not inspect.
        "neuradix-embedded-transport" => BTreeSet::new(),
        "neuradix-cli" => [
            "neuradix-contracts",
            "neuradix-time",
            "neuradix-runtime",
            "neuradix-record",
            "neuradix-safety",
            "neuradix-graph",
            "neuradix-studio",
        ]
        .into_iter()
        .collect(),
        "neuradix-testkit" => [
            "neuradix-contracts",
            "neuradix-time",
            "neuradix-runtime",
            "neuradix-transport-api",
        ]
        .into_iter()
        .collect(),
        other => panic!("unknown crate `{other}` in dependency-boundary test"),
    }
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/crates/testkit
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

/// Extract internal (`neuradix-*`) dependency names from the `[dependencies]`
/// section of a Cargo manifest, ignoring `[dev-dependencies]` and other tables.
fn internal_dependencies(manifest: &str) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]";
            continue;
        }
        if in_deps
            && trimmed.starts_with("neuradix-")
            && let Some(name) = trimmed.split(['=', ' ']).next()
        {
            deps.insert(name.to_owned());
        }
    }
    deps
}

#[test]
fn crate_dependencies_respect_the_layering() {
    let root = workspace_root();
    let crates = [
        "neuradix-contracts",
        "neuradix-time",
        "neuradix-transport-api",
        "neuradix-runtime",
        "neuradix-record",
        "neuradix-safety",
        "neuradix-python",
        "neuradix-graph",
        "neuradix-sim",
        "neuradix-studio",
        "neuradix-embedded-core",
        "neuradix-embedded-transport",
        "neuradix-cli",
        "neuradix-testkit",
    ];
    // Map crate name -> manifest path.
    let manifest_dirs = [
        ("neuradix-contracts", "crates/contracts"),
        ("neuradix-time", "crates/time"),
        ("neuradix-transport-api", "crates/transport-api"),
        ("neuradix-runtime", "crates/runtime"),
        ("neuradix-record", "crates/record"),
        ("neuradix-safety", "crates/safety"),
        ("neuradix-python", "crates/python"),
        ("neuradix-graph", "crates/graph"),
        ("neuradix-sim", "crates/sim"),
        ("neuradix-studio", "crates/studio"),
        ("neuradix-embedded-core", "crates/embedded-core"),
        ("neuradix-embedded-transport", "crates/embedded-transport"),
        ("neuradix-cli", "crates/cli"),
        ("neuradix-testkit", "crates/testkit"),
    ];

    for (name, dir) in manifest_dirs {
        assert!(crates.contains(&name));
        let manifest_path = root.join(dir).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
        let deps = internal_dependencies(&manifest);
        let permitted = allowed(name);
        for dep in &deps {
            assert!(
                permitted.contains(dep.as_str()),
                "`{name}` must not depend on `{dep}` (allowed: {permitted:?})"
            );
        }
    }
}
