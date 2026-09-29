//! The reference instance's checked-in inputs are exactly what the existing
//! generators and tooling produce, so the gateway exercises generated code and
//! a real configured artifact, not hand-written stand-ins.

use std::path::Path;

use neuradix_contracts::layout::WireLayout;
use neuradix_contracts::{ChannelEntry, ChannelManifest, validate};
use neuradix_embedded_codegen::{generate_channel_table, generate_nostd_rust};
use neuradix_gateway::reference;

fn contract(yaml: &str) -> neuradix_contracts::Contract {
    validate::from_yaml_str(yaml, Path::new("<t>")).unwrap()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

#[test]
fn g1_generated_decoders_match_the_generator() {
    for (yaml, file) in [
        (
            reference::VEHICLE_DEPTH_CONTRACT,
            "src/generated/vehicle_depth.rs",
        ),
        (
            reference::TINY_TELEMETRY_CONTRACT,
            "src/generated/tiny_telemetry.rs",
        ),
    ] {
        assert_eq!(
            generate_nostd_rust(&contract(yaml)).unwrap().code,
            read(file),
            "{file}"
        );
    }
}

#[test]
fn g2_configured_manifest_and_board_table_match_the_tooling() {
    let expected = ChannelManifest::new(vec![
        ChannelEntry::for_layout(
            1,
            "vehicle-depth",
            &WireLayout::for_contract(&contract(reference::VEHICLE_DEPTH_CONTRACT)).unwrap(),
        ),
        ChannelEntry::for_layout(
            2,
            "tiny-telemetry",
            &WireLayout::for_contract(&contract(reference::TINY_TELEMETRY_CONTRACT)).unwrap(),
        ),
    ])
    .unwrap();
    assert_eq!(reference::MANIFEST_JSON, expected.to_json_pretty());
    assert_eq!(
        expected.digest(),
        "sha256:4a8cf8f6ba9d3479fad842b80b177e0d90b93bcf259f070fd37e4ac9a00d97fb"
    );
    let table = generate_channel_table(&expected);
    assert_eq!(table, read("src/generated/channel_table.rs"));
    // The same bytes as the embedded-codegen golden board table.
    assert_eq!(
        table,
        read("../embedded-codegen/tests/golden/channel_table.rs")
    );
}

#[test]
fn g3_foreign_fixture_is_self_consistent_and_differs_only_on_channel_1() {
    let reference = ChannelManifest::parse(reference::MANIFEST_JSON).unwrap();
    let foreign = ChannelManifest::parse(&read("fixtures/foreign-manifest.json")).unwrap();
    let (r, f) = (reference.channels(), foreign.channels());
    assert_eq!(r[1], f[1]);
    assert_eq!(
        (r[0].compact_id, &r[0].name, r[0].wire_len),
        (f[0].compact_id, &f[0].name, f[0].wire_len)
    );
    assert_ne!(r[0].wire_id, f[0].wire_id);
    assert_ne!(reference.digest(), foreign.digest());
}
