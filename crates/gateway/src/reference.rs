//! The reference gateway instance and its simulated producer.
//!
//! Everything here is checked in or generated, never hand-written:
//!
//! | Item | Source |
//! | --- | --- |
//! | [`MANIFEST_JSON`] | configured artifact, `neuradix channel manifest crates/cli/tests/fixtures/channels/bindings.yaml` |
//! | [`generated::channel_table`] | `neuradix channel table` over the same bindings (equal to the embedded-codegen golden) |
//! | [`VehicleDepth`], [`TinyTelemetry`] | `neuradix contract generate --language nostd-rust` |
//! | expected contracts | `contracts/standard/navigation/vehicle-depth.yaml` and the tiny-telemetry codec fixture |
//!
//! `tests/generated.rs` regenerates each file and requires byte equality.
//! Tiny-telemetry is a codec conformance fixture, used here as a second,
//! differently shaped telemetry route; it is not a board application.

use neuradix_contracts::{Contract, validate};
use neuradix_embedded_core::DeploymentId;
use neuradix_embedded_transport::channel::{ENVELOPE_MAGIC, ENVELOPE_VERSION};
use neuradix_embedded_transport::{ChannelTable, ENVELOPE_HEADER, EnvelopeError, OVERHEAD, encode};

use crate::decoder::{Handle, Registration};
use crate::gateway::Gateway;
use crate::identity::{Attributed, IdentityClaims, IdentitySource};
use crate::routing::{BoardTable, ExpectedRoute, InitError, Route, RoutingTable};

/// Generated sources compiled into this build.
pub mod generated {
    /// Board channel table (`MANIFEST_DIGEST`, `CHANNELS`).
    pub mod channel_table {
        include!("generated/channel_table.rs");
    }
    /// Generated `VehicleDepth` payload type.
    pub mod vehicle_depth {
        include!("generated/vehicle_depth.rs");
    }
    /// Generated `TinyTelemetry` payload type.
    pub mod tiny_telemetry {
        include!("generated/tiny_telemetry.rs");
    }
}

pub use generated::tiny_telemetry::TinyTelemetry;
pub use generated::vehicle_depth::VehicleDepth;

crate::generated_decoder!(VehicleDepth);
crate::generated_decoder!(TinyTelemetry);

/// The configured manifest artifact.
pub const MANIFEST_JSON: &str = include_str!("../fixtures/reference-manifest.json");
/// Expected contract for channel 1.
pub const VEHICLE_DEPTH_CONTRACT: &str =
    include_str!("../../../contracts/standard/navigation/vehicle-depth.yaml");
/// Expected contract for channel 2.
pub const TINY_TELEMETRY_CONTRACT: &str =
    include_str!("../../embedded-codegen/tests/fixtures/tiny-telemetry.yaml");

/// Compact ID of `vehicle-depth`.
pub const VEHICLE_DEPTH_ID: u16 = 1;
/// Compact ID of `tiny-telemetry`.
pub const TINY_TELEMETRY_ID: u16 = 2;

/// Channel table capacity.
pub const CHANNEL_CAPACITY: usize = 8;
/// Frame payload buffer in bytes. The largest bound envelope is 8 + 29 = 37.
pub const FRAME_CAPACITY: usize = 64;

/// The reference gateway type.
pub type ReferenceGateway<H> = Gateway<H, CHANNEL_CAPACITY, FRAME_CAPACITY>;

/// The generated board table compiled into this build.
pub fn board_table() -> BoardTable {
    BoardTable {
        digest: generated::channel_table::MANIFEST_DIGEST,
        channels: &generated::channel_table::CHANNELS,
    }
}

/// Parse and validate an expected contract document.
pub fn contract(name: &str, yaml: &str) -> Result<Contract, InitError> {
    validate::from_yaml_str(yaml, std::path::Path::new(name)).map_err(|e| {
        InitError::InvalidContract {
            name: name.to_owned(),
            reason: e.to_string(),
        }
    })
}

/// The expected routes: `vehicle-depth` on 1 and `tiny-telemetry` on 2, each
/// with its generated decoder.
pub fn expected_routes<H>() -> Result<Vec<Route<H>>, InitError>
where
    H: Handle<VehicleDepth> + Handle<TinyTelemetry>,
{
    Ok(vec![
        Route::new(
            ExpectedRoute::from_contract(
                VEHICLE_DEPTH_ID,
                "vehicle-depth",
                &contract("vehicle-depth", VEHICLE_DEPTH_CONTRACT)?,
            )?,
            Registration::of::<VehicleDepth>(),
        ),
        Route::new(
            ExpectedRoute::from_contract(
                TINY_TELEMETRY_ID,
                "tiny-telemetry",
                &contract("tiny-telemetry", TINY_TELEMETRY_CONTRACT)?,
            )?,
            Registration::of::<TinyTelemetry>(),
        ),
    ])
}

/// Build the reference gateway from a configured manifest.
pub fn gateway<H>(manifest_json: &str) -> Result<ReferenceGateway<H>, InitError>
where
    H: Handle<VehicleDepth> + Handle<TinyTelemetry>,
{
    Gateway::new(RoutingTable::new(
        manifest_json,
        board_table(),
        expected_routes()?,
    )?)
}

/// Simulated producer node name.
pub const PRODUCER_NODE: &str = "sim-telemetry-node";
/// Simulated producer firmware identity.
pub const PRODUCER_FIRMWARE: &str = "sim-producer 0.0.1 (host simulation, not firmware)";
/// Simulated deployment identity: a fixed placeholder, not a resolved
/// deployment digest.
pub const PRODUCER_DEPLOYMENT: DeploymentId = DeploymentId::from_bytes([0x5a; 32]);

/// The simulated producer's identities, attributed as fixtures.
pub fn simulated_claims() -> IdentityClaims {
    IdentityClaims {
        node: Some(Attributed::new(
            PRODUCER_NODE.to_owned(),
            IdentitySource::SimulatedFixture,
        )),
        firmware: Some(Attributed::new(
            PRODUCER_FIRMWARE.to_owned(),
            IdentitySource::SimulatedFixture,
        )),
        deployment: Some(Attributed::new(
            PRODUCER_DEPLOYMENT,
            IdentitySource::SimulatedFixture,
        )),
    }
}

/// A host-simulated producer: generated encoders, the compact envelope and the
/// CRC frame, exactly as a board would emit them.
#[derive(Debug, Clone)]
pub struct SimulatedProducer<const N: usize> {
    table: ChannelTable<N>,
    seq: u16,
}

impl SimulatedProducer<CHANNEL_CAPACITY> {
    /// A producer provisioned with the generated reference board table.
    pub fn reference() -> Result<Self, InitError> {
        let board = board_table();
        ChannelTable::new(board.digest, board.channels)
            .map(Self::new)
            .map_err(InitError::BoardTable)
    }
}

impl<const N: usize> SimulatedProducer<N> {
    /// A producer provisioned with `table`.
    pub fn new(table: ChannelTable<N>) -> Self {
        Self { table, seq: 0 }
    }

    /// Frame a `VehicleDepth` on `compact_id`.
    pub fn vehicle_depth(&mut self, compact_id: u16, v: &VehicleDepth) -> Vec<u8> {
        let mut body = [0; VehicleDepth::WIRE_LEN];
        v.encode(&mut body);
        self.channel(compact_id, &body)
            .expect("vehicle-depth body is bound")
    }

    /// Frame a `TinyTelemetry` on `compact_id`.
    pub fn tiny_telemetry(&mut self, compact_id: u16, v: &TinyTelemetry) -> Vec<u8> {
        let mut body = [0; TinyTelemetry::WIRE_LEN];
        v.encode(&mut body);
        self.channel(compact_id, &body)
            .expect("tiny-telemetry body is bound")
    }

    /// Seal `body` on `compact_id` and frame it.
    pub fn channel(&mut self, compact_id: u16, body: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
        let mut envelope = vec![0; ENVELOPE_HEADER + body.len()];
        let n = self.table.seal(compact_id, body, &mut envelope)?;
        envelope.truncate(n);
        Ok(self.frame(&envelope))
    }

    /// An envelope with explicit header fields (for adversarial traffic).
    pub fn raw_envelope(compact_id: u16, tag: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = vec![ENVELOPE_MAGIC, ENVELOPE_VERSION];
        out.extend_from_slice(&compact_id.to_le_bytes());
        out.extend_from_slice(&tag);
        out.extend_from_slice(body);
        out
    }

    /// Frame an arbitrary payload under the next sequence number.
    pub fn frame(&mut self, payload: &[u8]) -> Vec<u8> {
        let mut wire = vec![0; OVERHEAD + payload.len()];
        let n = encode(self.seq, payload, &mut wire).expect("buffer sized for the payload");
        self.seq = self.seq.wrapping_add(1);
        wire.truncate(n);
        wire
    }

    /// The producer's manifest tag.
    pub fn manifest_tag(&self) -> [u8; 4] {
        self.table.manifest_tag()
    }
}
