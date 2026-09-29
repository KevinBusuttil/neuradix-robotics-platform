//! Golden-file regression for the embedded projections.
//!
//! Run with `UPDATE_GOLDEN=1` to (re)write the checked-in golden files; without
//! it, the test asserts the generators still produce byte-identical output.

use std::path::PathBuf;

use neuradix_contracts::layout::WireLayout;
use neuradix_contracts::{ChannelEntry, ChannelManifest};
use neuradix_contracts::{Contract, load_file};
use neuradix_embedded_codegen::{
    generate_channel_table, generate_cpp, generate_nostd_rust, golden_vectors,
};

fn contract() -> Contract {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/standard/navigation/vehicle-depth.yaml");
    load_file(&path).expect("load vehicle-depth contract")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Assert `actual` equals the checked-in golden `name`, or write it when
/// `UPDATE_GOLDEN` is set.
fn check(name: &str, actual: &str) {
    let path = golden_dir().join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read golden {}: {e} (run with UPDATE_GOLDEN=1)",
            path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "golden drift in {name}; run with UPDATE_GOLDEN=1 to update"
    );
}

#[test]
fn projections_match_golden_files() {
    let c = contract();
    check("vehicle_depth.rs", &generate_nostd_rust(&c).unwrap().code);
    check("vehicle_depth.h", &generate_cpp(&c).unwrap().code);
    let vectors = serde_json::to_string_pretty(&golden_vectors(&c).unwrap()).unwrap() + "\n";
    check("vehicle_depth_vectors.json", &vectors);
}

/// Board channel table for vehicle depth (compact 1) and the tiny telemetry
/// fixture (compact 2). `channel_binding.rs` compiles and verifies this file.
#[test]
fn channel_table_matches_golden_file() {
    let tiny = load_file(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny-telemetry.yaml"),
    )
    .unwrap();
    let manifest = ChannelManifest::new(vec![
        ChannelEntry::for_layout(
            1,
            "vehicle-depth",
            &WireLayout::for_contract(&contract()).unwrap(),
        ),
        ChannelEntry::for_layout(
            2,
            "tiny-telemetry",
            &WireLayout::for_contract(&tiny).unwrap(),
        ),
    ])
    .unwrap();
    check("channel_table.rs", &generate_channel_table(&manifest));
}
