//! Shared test support: a recording handler and the reference instance.
#![allow(dead_code)]

use neuradix_embedded_transport::ChannelBinding;
use neuradix_gateway::Handle;
use neuradix_gateway::reference::{
    self, ReferenceGateway, SimulatedProducer, TinyTelemetry, VehicleDepth,
};

/// Records every typed value with its resolved compact ID.
#[derive(Debug, Default)]
pub struct Recorder {
    pub depth: Vec<(u16, VehicleDepth)>,
    pub tiny: Vec<(u16, TinyTelemetry)>,
}

impl Recorder {
    pub fn calls(&self) -> usize {
        self.depth.len() + self.tiny.len()
    }
}

impl Handle<VehicleDepth> for Recorder {
    fn handle(&mut self, channel: &ChannelBinding, value: VehicleDepth) {
        self.depth.push((channel.compact_id, value));
    }
}

impl Handle<TinyTelemetry> for Recorder {
    fn handle(&mut self, channel: &ChannelBinding, value: TinyTelemetry) {
        self.tiny.push((channel.compact_id, value));
    }
}

pub type Producer = SimulatedProducer<{ reference::CHANNEL_CAPACITY }>;

pub fn gateway() -> ReferenceGateway<Recorder> {
    reference::gateway(reference::MANIFEST_JSON).expect("reference configuration verifies")
}

pub fn producer() -> Producer {
    SimulatedProducer::reference().expect("generated board table verifies")
}

pub fn depth(d: f64) -> VehicleDepth {
    VehicleDepth {
        depth: d,
        uncertainty: 0.5,
    }
}

pub fn tiny(n: u32) -> TinyTelemetry {
    TinyTelemetry {
        a_enabled: n.is_multiple_of(2),
        b_measurement: n as f32 * 0.5,
        c_signed32: -(n as i32),
        d_signed64: i64::MIN + i64::from(n),
        e_unsigned32: n,
        z_unsigned64: u64::MAX - u64::from(n),
    }
}

/// The reference manifest with its declared digest replaced.
pub fn with_digest(json: &str, digest: &str) -> String {
    let start = json.find("\"digest\": \"").expect("digest field") + "\"digest\": \"".len();
    let end = start + json[start..].find('"').expect("digest end");
    format!("{}{digest}{}", &json[..start], &json[end..])
}
