//! WP-A02 end to end: verified channel manifest -> no_std channel table -> CRC
//! frame -> generated decoder. The receiver takes the peer wire ID from the
//! manifest binding, never its own constant, so neither a compact-ID remap nor
//! an equal-length foreign payload decodes silently.
mod common;

use std::path::Path;

use neuradix_contracts::layout::WireLayout;
use neuradix_contracts::{ChannelEntry, ChannelManifest, ChannelManifestError, validate};
use neuradix_embedded_transport::{
    ChannelBinding, ChannelTable, ENVELOPE_HEADER, EnvelopeError, FrameDecoder, FrameEvent,
    OVERHEAD, encode,
};

include!("golden/vehicle_depth.rs");

/// A 16-byte contract with a different identity from vehicle depth.
fn range() -> WireLayout {
    let yaml = "apiVersion: contracts.neuradix.io/v1alpha1
kind: StreamContract
metadata:
  namespace: io.neuradix.test
  name: range
  version: 1.0.0
spec:
  description: test
  payload:
    type: object
    fields:
      altitude: { type: float64 }
      range: { type: float64 }
  semantics:
    frame: vehicle/base
    clockDomain: monotonic
    authoritativeTimestamp: measurement
    maximumAge: 100ms
  delivery:
    capacity: 8
    overflow: keep-latest
";
    WireLayout::for_contract(&validate::from_yaml_str(yaml, Path::new("<t>")).unwrap()).unwrap()
}

fn depth() -> WireLayout {
    WireLayout::for_contract(&common::depth()).unwrap()
}

/// What a board build would compile in from the verified manifest.
fn table(json: &str) -> ChannelTable<4> {
    let m = ChannelManifest::parse(json).unwrap();
    let bindings: Vec<_> = m
        .channels()
        .iter()
        .map(|c| ChannelBinding {
            compact_id: c.compact_id,
            wire_id: Box::leak(c.wire_id.clone().into_boxed_str()),
            wire_len: u16::try_from(c.wire_len).unwrap(),
        })
        .collect();
    ChannelTable::new(m.digest_bytes(), &bindings).unwrap()
}

fn transmit(t: &ChannelTable<4>, id: u16, body: &[u8]) -> Vec<u8> {
    let mut env = vec![0; ENVELOPE_HEADER + body.len()];
    t.seal(id, body, &mut env).unwrap();
    let mut wire = vec![0; OVERHEAD + env.len()];
    let n = encode(1, &env, &mut wire).unwrap();
    wire.truncate(n);
    wire
}

fn receive(t: &ChannelTable<4>, wire: &[u8]) -> Result<Option<VehicleDepth>, EnvelopeError> {
    let mut dec = FrameDecoder::<64>::new();
    for &byte in wire {
        if let Some(FrameEvent::Frame(_)) = dec.push(byte) {
            let e = t.open(dec.payload())?;
            return Ok(VehicleDepth::decode(e.body, e.binding.wire_id));
        }
    }
    panic!("no frame")
}

fn manifest(depth_id: u16, range_id: u16) -> String {
    ChannelManifest::new(vec![
        ChannelEntry::for_layout(depth_id, "depth", &depth()),
        ChannelEntry::for_layout(range_id, "range", &range()),
    ])
    .unwrap()
    .to_json_pretty()
}

#[test]
fn e1_bound_channel_decodes() {
    let json = manifest(1, 2);
    let m = ChannelManifest::parse(&json).unwrap();
    // Receiver's generated decoder is built from the layout bound to channel 1.
    assert_eq!(
        m.verify_layout(1, &depth()).unwrap().wire_id,
        VehicleDepth::WIRE_ID
    );
    let t = table(&json);
    let v = VehicleDepth {
        depth: 12.5,
        uncertainty: 0.25,
    };
    let mut body = [0; VehicleDepth::WIRE_LEN];
    v.encode(&mut body).unwrap();
    assert_eq!(receive(&t, &transmit(&t, 1, &body)), Ok(Some(v)));
}

#[test]
fn e2_equal_length_foreign_channel_does_not_decode() {
    let t = table(&manifest(1, 2));
    let body = [0x3F; 16];
    // Envelope resolves channel 2 correctly, but its wire ID is `range`, so the
    // vehicle-depth decoder refuses it despite the identical length.
    assert_eq!(receive(&t, &transmit(&t, 2, &body)), Ok(None));
    let m = ChannelManifest::parse(&manifest(1, 2)).unwrap();
    assert!(matches!(
        m.verify_layout(2, &depth()),
        Err(ChannelManifestError::LayoutMismatch { .. })
    ));
}

#[test]
fn e3_remapped_peer_is_rejected_before_decode() {
    // Producer re-provisioned: compact 1 now carries `range`.
    let producer = table(&manifest(2, 1));
    let receiver = table(&manifest(1, 2));
    let wire = transmit(&producer, 1, &[0x3F; 16]);
    assert_eq!(
        receive(&receiver, &wire),
        Err(EnvelopeError::ManifestMismatch)
    );
}

#[test]
fn e4_collision_and_legacy_codec_manifests_are_rejected() {
    assert!(matches!(
        ChannelManifest::new(vec![
            ChannelEntry::for_layout(1, "depth", &depth()),
            ChannelEntry::for_layout(1, "range", &range()),
        ]),
        Err(ChannelManifestError::CompactIdCollision { compact_id: 1, .. })
    ));
    let mut legacy = ChannelEntry::for_layout(1, "depth", &depth());
    legacy.codec_id = "neuradix.scalar-le.v1".into();
    assert!(matches!(
        ChannelManifest::new(vec![legacy]),
        Err(ChannelManifestError::UnsupportedCodec { .. })
    ));
}

#[test]
fn e5_unenveloped_legacy_frame_is_rejected() {
    let t = table(&manifest(1, 2));
    let mut body = [0; 16];
    VehicleDepth {
        depth: 1.0,
        uncertainty: 2.0,
    }
    .encode(&mut body)
    .unwrap();
    let mut wire = vec![0; OVERHEAD + 16];
    encode(1, &body, &mut wire).unwrap();
    assert_eq!(body[0], 0);
    assert_eq!(receive(&t, &wire), Err(EnvelopeError::BadMagic));
}
