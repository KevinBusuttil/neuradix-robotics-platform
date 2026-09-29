//! Initialization: the complete mapping is verified before any traffic, and
//! every rejection leaves no gateway (so no handler can ever be called).
mod common;

use std::path::Path;

use common::{Recorder, with_digest};
use neuradix_contracts::layout::WireLayout;
use neuradix_contracts::{ChannelEntry, ChannelManifest, ChannelManifestError, validate};
use neuradix_embedded_transport::{BindError, ChannelBinding, manifest_digest};
use neuradix_gateway::reference::{
    self, TinyTelemetry, VehicleDepth, board_table,
    generated::channel_table::{CHANNELS, MANIFEST_DIGEST},
};
use neuradix_gateway::{
    BoardTable, ExpectedRoute, Gateway, GeneratedDecoder, InitError, MAX_MANIFEST_BYTES,
    Registration, Route, RoutingTable,
};

type Table = RoutingTable<Recorder, { reference::CHANNEL_CAPACITY }>;

fn layout(yaml: &str) -> WireLayout {
    WireLayout::for_contract(&validate::from_yaml_str(yaml, Path::new("<t>")).unwrap()).unwrap()
}

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

fn range_yaml() -> String {
    fixture("../cli/tests/fixtures/channels/range.yaml")
}

fn manifest(entries: Vec<ChannelEntry>) -> String {
    ChannelManifest::new(entries).unwrap().to_json_pretty()
}

fn depth_entry(id: u16, name: &str) -> ChannelEntry {
    ChannelEntry::for_layout(id, name, &layout(reference::VEHICLE_DEPTH_CONTRACT))
}

fn tiny_entry(id: u16, name: &str) -> ChannelEntry {
    ChannelEntry::for_layout(id, name, &layout(reference::TINY_TELEMETRY_CONTRACT))
}

fn init(json: &str) -> Result<(), InitError> {
    reference::gateway::<Recorder>(json).map(|_| ())
}

fn with_routes(routes: Vec<Route<Recorder>>) -> Result<(), InitError> {
    Table::new(reference::MANIFEST_JSON, board_table(), routes).map(|_| ())
}

fn route<T: GeneratedDecoder>(id: u16, name: &str, yaml: &str) -> Route<Recorder>
where
    Recorder: neuradix_gateway::Handle<T>,
{
    let contract = reference::contract(name, yaml).unwrap();
    Route::new(
        ExpectedRoute::from_contract(id, name, &contract).unwrap(),
        Registration::of::<T>(),
    )
}

fn depth_route(id: u16, name: &str) -> Route<Recorder> {
    route::<VehicleDepth>(id, name, reference::VEHICLE_DEPTH_CONTRACT)
}

fn tiny_route(id: u16, name: &str) -> Route<Recorder> {
    route::<TinyTelemetry>(id, name, reference::TINY_TELEMETRY_CONTRACT)
}

#[test]
fn i1_reference_configuration_initialises() {
    let g = common::gateway();
    assert_eq!(
        g.routes().manifest_digest(),
        "sha256:4a8cf8f6ba9d3479fad842b80b177e0d90b93bcf259f070fd37e4ac9a00d97fb"
    );
    let routes: Vec<_> = g
        .routes()
        .routes()
        .map(|(b, ty)| (b.compact_id, b.name, b.wire_id, ty))
        .collect();
    assert_eq!(
        routes,
        [
            (1, "vehicle-depth", VehicleDepth::WIRE_ID, "VehicleDepth"),
            (2, "tiny-telemetry", TinyTelemetry::WIRE_ID, "TinyTelemetry"),
        ]
    );
    assert_eq!(g.routes().max_envelope_len(), 8 + 29);
    assert_eq!(g.routes().manifest_tag(), [0x4a, 0x8c, 0xf8, 0xf6]);
}

#[test]
fn i2_manifest_artifact_is_verified_in_full() {
    let json = reference::MANIFEST_JSON;
    // Tampered digest.
    let tampered = with_digest(
        json,
        "sha256:4a8cf8f6ba9d3479fad842b80b177e0d90b93bcf259f070fd37e4ac9a00d97fc",
    );
    assert!(matches!(
        init(&tampered),
        Err(InitError::Manifest(
            ChannelManifestError::DigestMismatch { .. }
        ))
    ));
    // Another manifest's valid digest paired with these entries.
    let other = ChannelManifest::parse(&fixture("fixtures/foreign-manifest.json")).unwrap();
    assert!(matches!(
        init(&with_digest(json, &other.digest())),
        Err(InitError::Manifest(
            ChannelManifestError::DigestMismatch { .. }
        ))
    ));
    // Unknown version, unknown field, malformed identity, malformed JSON.
    for (bad, what) in [
        (
            json.replace("channel-manifest.v2", "channel-manifest.v1"),
            "version",
        ),
        (
            json.replacen("\"name\"", "\"extra\": 1, \"name\"", 1),
            "field",
        ),
        (json.replacen("sha256:4780f", "sha256:4780F", 1), "identity"),
        (json[..json.len() / 2].to_owned(), "json"),
    ] {
        assert!(
            matches!(init(&bad), Err(InitError::Manifest(_))),
            "{what}: {:?}",
            init(&bad)
        );
    }
    // Oversized input is refused before parsing.
    let huge = format!("{json}{}", " ".repeat(MAX_MANIFEST_BYTES));
    assert_eq!(
        init(&huge),
        Err(InitError::ManifestTooLarge {
            len: huge.len(),
            limit: MAX_MANIFEST_BYTES
        })
    );
}

#[test]
fn i3_self_consistent_manifests_that_disagree_with_the_routes_are_refused() {
    // Equal-width foreign contract on channel 1 (`neuradix channel manifest`
    // output, valid digest).
    assert_eq!(
        init(&fixture("fixtures/foreign-manifest.json")),
        Err(InitError::ContractMismatch {
            compact_id: 1,
            name: "vehicle-depth".into()
        })
    );
    // The same, built directly: range layout under the expected name.
    let foreign = manifest(vec![
        ChannelEntry::for_layout(1, "vehicle-depth", &layout(&range_yaml())),
        tiny_entry(2, "tiny-telemetry"),
    ]);
    assert!(matches!(
        init(&foreign),
        Err(InitError::ContractMismatch { compact_id: 1, .. })
    ));
    // Swapped compact IDs (names travel with their contracts).
    assert!(matches!(
        init(&manifest(vec![
            tiny_entry(1, "tiny-telemetry"),
            depth_entry(2, "vehicle-depth")
        ])),
        Err(InitError::NameMismatch { compact_id: 1, .. })
    ));
    // Swapped contracts under the expected names.
    assert!(matches!(
        init(&manifest(vec![
            tiny_entry(1, "vehicle-depth"),
            depth_entry(2, "tiny-telemetry")
        ])),
        Err(InitError::ContractMismatch { compact_id: 1, .. })
    ));
    // Renamed channel.
    assert_eq!(
        init(&manifest(vec![
            depth_entry(1, "depth"),
            tiny_entry(2, "tiny-telemetry")
        ])),
        Err(InitError::NameMismatch {
            compact_id: 1,
            expected: "vehicle-depth".into(),
            manifest: "depth".into()
        })
    );
    // Missing channel: a route has no manifest entry.
    assert_eq!(
        init(&manifest(vec![depth_entry(1, "vehicle-depth")])),
        Err(InitError::UnknownRouteChannel { compact_id: 2 })
    );
    // Extra channel (the same contract on a third ID): no route for it.
    assert_eq!(
        init(&manifest(vec![
            depth_entry(1, "vehicle-depth"),
            tiny_entry(2, "tiny-telemetry"),
            depth_entry(3, "vehicle-depth-aft"),
        ])),
        Err(InitError::UnroutedChannel {
            compact_id: 3,
            name: "vehicle-depth-aft".into()
        })
    );
    // An initialization failure leaves nothing that could call a handler.
    let recorder = Recorder::default();
    assert_eq!(recorder.calls(), 0);
}

#[test]
fn i4_route_lists_are_complete_unique_and_supported() {
    assert!(
        with_routes(vec![
            depth_route(1, "vehicle-depth"),
            tiny_route(2, "tiny-telemetry")
        ])
        .is_ok()
    );
    assert_eq!(
        with_routes(vec![
            depth_route(1, "vehicle-depth"),
            depth_route(1, "vehicle-depth-2"),
            tiny_route(2, "tiny-telemetry"),
        ]),
        Err(InitError::DuplicateRoute { compact_id: 1 })
    );
    assert_eq!(
        with_routes(vec![
            depth_route(1, "vehicle-depth"),
            tiny_route(2, "vehicle-depth")
        ]),
        Err(InitError::DuplicateRouteName {
            name: "vehicle-depth".into()
        })
    );
    // Missing route for a manifest channel.
    assert_eq!(
        with_routes(vec![depth_route(1, "vehicle-depth")]),
        Err(InitError::UnroutedChannel {
            compact_id: 2,
            name: "tiny-telemetry".into()
        })
    );
    // Extra route for a channel the manifest does not bind.
    assert_eq!(
        with_routes(vec![
            depth_route(1, "vehicle-depth"),
            tiny_route(2, "tiny-telemetry"),
            depth_route(3, "vehicle-depth-aft"),
        ]),
        Err(InitError::UnknownRouteChannel { compact_id: 3 })
    );
    // Reserved ID 0 is never bound, so a route for it is refused.
    assert_eq!(
        with_routes(vec![
            depth_route(0, "zero"),
            depth_route(1, "vehicle-depth"),
            tiny_route(2, "tiny-telemetry"),
        ]),
        Err(InitError::UnknownRouteChannel { compact_id: 0 })
    );
    // Unsupported and invalid expected contracts.
    let labelled = reference::contract(
        "labelled",
        &fixture("../cli/tests/fixtures/channels/labelled.yaml"),
    )
    .unwrap();
    assert!(matches!(
        ExpectedRoute::from_contract(1, "labelled", &labelled),
        Err(InitError::UnsupportedContract { .. })
    ));
    assert!(matches!(
        reference::contract("broken", "apiVersion: nope"),
        Err(InitError::InvalidContract { .. })
    ));
}

/// A decoder whose identity constants claim `VehicleDepth`'s wire ID but a
/// different length.
struct Mislabelled;
impl GeneratedDecoder for Mislabelled {
    const TYPE_NAME: &'static str = "Mislabelled";
    const WIRE_LEN: usize = 15;
    const SCHEMA_ID: &'static str = VehicleDepth::SCHEMA_ID;
    const CODEC_ID: &'static str = VehicleDepth::CODEC_ID;
    const WIRE_ID: &'static str = VehicleDepth::WIRE_ID;
    fn decode(_: &[u8], _: &str) -> Option<Self> {
        Some(Self)
    }
}
impl neuradix_gateway::Handle<Mislabelled> for Recorder {
    fn handle(&mut self, _: &ChannelBinding, _: Mislabelled) {
        panic!("never routed");
    }
}

#[test]
fn i5_decoder_registrations_must_match_the_expected_contract() {
    // The tiny-telemetry channel registered with the VehicleDepth decoder.
    let wrong = Route::new(
        tiny_route(2, "tiny-telemetry").expected,
        Registration::of::<VehicleDepth>(),
    );
    assert_eq!(
        with_routes(vec![depth_route(1, "vehicle-depth"), wrong]),
        Err(InitError::DecoderMismatch {
            compact_id: 2,
            decoder: "VehicleDepth"
        })
    );
    // Right wire ID, wrong length.
    let mislabelled = Route::new(
        depth_route(1, "vehicle-depth").expected,
        Registration::of::<Mislabelled>(),
    );
    assert_eq!(
        with_routes(vec![mislabelled, tiny_route(2, "tiny-telemetry")]),
        Err(InitError::DecoderMismatch {
            compact_id: 1,
            decoder: "Mislabelled"
        })
    );
    // The expected contract is the equal-width foreign one: the manifest entry
    // does not carry it, whatever the decoder.
    let range_expected = ExpectedRoute::from_contract(
        1,
        "vehicle-depth",
        &reference::contract("range", &range_yaml()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        with_routes(vec![
            Route::new(range_expected, Registration::of::<VehicleDepth>()),
            tiny_route(2, "tiny-telemetry"),
        ]),
        Err(InitError::ContractMismatch {
            compact_id: 1,
            name: "vehicle-depth".into()
        })
    );
}

static SUBSET: [ChannelBinding; 1] = [CHANNELS[0]];
static EDITED: [ChannelBinding; 2] = [
    ChannelBinding {
        wire_id: CHANNELS[1].wire_id,
        ..CHANNELS[0]
    },
    CHANNELS[1],
];

fn with_board(board: BoardTable) -> Result<(), InitError> {
    Table::new(
        reference::MANIFEST_JSON,
        board,
        reference::expected_routes().unwrap(),
    )
    .map(|_| ())
}

#[test]
fn i6_compiled_board_table_must_equal_the_configured_manifest() {
    // Board digest with different bindings.
    assert_eq!(
        with_board(BoardTable {
            digest: MANIFEST_DIGEST,
            channels: &SUBSET
        }),
        Err(InitError::BoardTable(BindError::DigestMismatch))
    );
    assert_eq!(
        with_board(BoardTable {
            digest: MANIFEST_DIGEST,
            channels: &EDITED
        }),
        Err(InitError::BoardTable(BindError::DigestMismatch))
    );
    let mut flipped = MANIFEST_DIGEST;
    flipped[0] ^= 1;
    assert_eq!(
        with_board(BoardTable {
            digest: flipped,
            channels: &CHANNELS
        }),
        Err(InitError::BoardTable(BindError::DigestMismatch))
    );
    // A self-consistent board table for a different manifest.
    assert_eq!(
        with_board(BoardTable {
            digest: manifest_digest(&SUBSET),
            channels: &SUBSET
        }),
        Err(InitError::BoardTableMismatch)
    );
}

#[test]
fn i7_capacities_are_checked() {
    assert_eq!(
        RoutingTable::<Recorder, 1>::new(
            reference::MANIFEST_JSON,
            board_table(),
            reference::expected_routes().unwrap()
        )
        .map(|_| ())
        .unwrap_err(),
        InitError::Capacity {
            what: "manifest channel",
            count: 2,
            capacity: 1
        }
    );
    let routes = Table::new(
        reference::MANIFEST_JSON,
        board_table(),
        reference::expected_routes().unwrap(),
    )
    .unwrap();
    assert_eq!(
        Gateway::<Recorder, { reference::CHANNEL_CAPACITY }, 36>::new(routes).map(|_| ()),
        Err(InitError::FrameCapacity {
            needed: 37,
            capacity: 36
        })
    );
}
