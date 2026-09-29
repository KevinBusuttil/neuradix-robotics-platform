//! WP-A02 verified channel manifest: compact IDs resolve to full wire identity.
use std::path::Path;

use neuradix_contracts::channel::{CHANNEL_MANIFEST_VERSION, MAX_CHANNEL_WIRE_LEN};
use neuradix_contracts::layout::WireLayout;
use neuradix_contracts::{ChannelEntry, ChannelManifest, ChannelManifestError, validate};

fn layout(name: &str, fields: &[(&str, &str)]) -> WireLayout {
    let fields: String = fields
        .iter()
        .map(|(f, ty)| format!("      {f}: {{ type: {ty} }}\n"))
        .collect();
    let yaml = format!(
        "apiVersion: contracts.neuradix.io/v1alpha1
kind: StreamContract
metadata:
  namespace: io.neuradix.test
  name: {name}
  version: 1.0.0
spec:
  description: test
  payload:
    type: object
    fields:
{fields}  semantics:
    frame: vehicle/base
    clockDomain: monotonic
    authoritativeTimestamp: measurement
    maximumAge: 100ms
  delivery:
    capacity: 8
    overflow: keep-latest
"
    );
    WireLayout::for_contract(&validate::from_yaml_str(&yaml, Path::new("<t>")).unwrap()).unwrap()
}

fn depth() -> WireLayout {
    layout("depth", &[("depth", "float64"), ("uncertainty", "float64")])
}
/// Same 16-byte length as `depth`, different identity.
fn range() -> WireLayout {
    layout("range", &[("altitude", "float64"), ("range", "float64")])
}

fn manifest() -> ChannelManifest {
    ChannelManifest::new(vec![
        ChannelEntry::for_layout(2, "range", &range()),
        ChannelEntry::for_layout(1, "depth", &depth()),
    ])
    .unwrap()
}

#[test]
fn m1_round_trip_is_verified_and_order_independent() {
    let m = manifest();
    assert_eq!(m.channels()[0].compact_id, 1);
    let json = m.to_json_pretty();
    assert!(json.contains(CHANNEL_MANIFEST_VERSION));
    let parsed = ChannelManifest::parse(&json).unwrap();
    assert_eq!(parsed, m);
    let reordered = ChannelManifest::new(vec![
        ChannelEntry::for_layout(1, "depth", &depth()),
        ChannelEntry::for_layout(2, "range", &range()),
    ])
    .unwrap();
    assert_eq!(reordered.digest(), m.digest());
    assert_eq!(m.digest().len(), 71);
    assert_eq!(&m.digest_bytes()[..], &hex(&m.digest()[7..])[..]);
    assert_eq!(m.resolve(1).unwrap().wire_id, depth().wire_id);
    assert_eq!(m.resolve(3), None);
}

#[test]
fn m2_compact_id_collisions_are_rejected() {
    let err = ChannelManifest::new(vec![
        ChannelEntry::for_layout(7, "depth", &depth()),
        ChannelEntry::for_layout(7, "range", &range()),
    ])
    .unwrap_err();
    assert_eq!(
        err,
        ChannelManifestError::CompactIdCollision {
            compact_id: 7,
            first: "depth".into(),
            second: "range".into()
        }
    );
    // Even identical identities may not share a compact ID.
    let err = ChannelManifest::new(vec![
        ChannelEntry::for_layout(7, "a", &depth()),
        ChannelEntry::for_layout(7, "b", &depth()),
    ])
    .unwrap_err();
    assert!(matches!(
        err,
        ChannelManifestError::CompactIdCollision { compact_id: 7, .. }
    ));
    // One layout on two compact IDs is two channels, not a collision.
    ChannelManifest::new(vec![
        ChannelEntry::for_layout(1, "a", &depth()),
        ChannelEntry::for_layout(2, "b", &depth()),
    ])
    .unwrap();
}

#[test]
fn m3_entry_rules() {
    let bad = |edit: &dyn Fn(&mut ChannelEntry)| {
        let mut e = ChannelEntry::for_layout(1, "depth", &depth());
        edit(&mut e);
        ChannelManifest::new(vec![e]).unwrap_err()
    };
    assert_eq!(
        bad(&|e| e.compact_id = 0),
        ChannelManifestError::ReservedCompactId {
            name: "depth".into()
        }
    );
    assert!(matches!(
        bad(&|e| e.codec_id = "neuradix.scalar-le.v1".into()),
        ChannelManifestError::UnsupportedCodec { .. }
    ));
    assert!(matches!(
        bad(&|e| e.codec_id = "legacy-unversioned".into()),
        ChannelManifestError::UnsupportedCodec { .. }
    ));
    for wire_id in [
        String::new(),
        depth().wire_id[..20].to_owned(),
        depth().wire_id.to_uppercase(),
        depth().wire_id.replace("sha256:", "sha512:"),
        format!("{}0", depth().wire_id),
    ] {
        let w = wire_id.clone();
        assert!(
            matches!(
                bad(&move |e| e.wire_id = w.clone()),
                ChannelManifestError::MalformedIdentity { .. }
            ),
            "{wire_id}"
        );
    }
    assert!(matches!(
        bad(&|e| e.schema_id = "io.neuradix.test/depth".into()),
        ChannelManifestError::MalformedIdentity { .. }
    ));
    assert!(matches!(
        bad(&|e| e.wire_len = 0),
        ChannelManifestError::InvalidWireLen { .. }
    ));
    assert!(matches!(
        bad(&|e| e.wire_len = MAX_CHANNEL_WIRE_LEN + 1),
        ChannelManifestError::InvalidWireLen { .. }
    ));
    assert!(matches!(
        bad(&|e| e.name = String::new()),
        ChannelManifestError::InvalidName(_)
    ));
    assert!(matches!(
        bad(&|e| e.name = "a\nb".into()),
        ChannelManifestError::InvalidName(_)
    ));
    assert_eq!(
        ChannelManifest::new(vec![]).unwrap_err(),
        ChannelManifestError::Empty
    );
    assert_eq!(
        ChannelManifest::new(vec![
            ChannelEntry::for_layout(1, "x", &depth()),
            ChannelEntry::for_layout(2, "x", &range()),
        ])
        .unwrap_err(),
        ChannelManifestError::DuplicateName("x".into())
    );
}

#[test]
fn m4_parse_rejects_tampering_and_unknown_shapes() {
    let json = manifest().to_json_pretty();
    // Rebinding channel 1 to another identity without a new digest.
    let tampered = json.replacen(&depth().wire_id, &range().wire_id, 1);
    assert!(matches!(
        ChannelManifest::parse(&tampered).unwrap_err(),
        ChannelManifestError::DigestMismatch { .. }
    ));
    // Swapping two compact IDs is also a different manifest.
    let swapped = json
        .replace("\"compact_id\": 1", "\"compact_id\": 9")
        .replace("\"compact_id\": 2", "\"compact_id\": 1")
        .replace("\"compact_id\": 9", "\"compact_id\": 2");
    assert!(matches!(
        ChannelManifest::parse(&swapped).unwrap_err(),
        ChannelManifestError::DigestMismatch { .. }
    ));
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v["manifest_version"] = "neuradix.channel-manifest.v0".into();
    assert!(matches!(
        ChannelManifest::parse(&v.to_string()).unwrap_err(),
        ChannelManifestError::UnsupportedVersion(_)
    ));
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v["channels"][0]["session"] = 1.into();
    assert!(matches!(
        ChannelManifest::parse(&v.to_string()).unwrap_err(),
        ChannelManifestError::Parse(_)
    ));
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v.as_object_mut().unwrap().remove("digest");
    assert!(matches!(
        ChannelManifest::parse(&v.to_string()).unwrap_err(),
        ChannelManifestError::Parse(_)
    ));
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v["channels"][1]["compact_id"] = 1.into();
    assert!(matches!(
        ChannelManifest::parse(&v.to_string()).unwrap_err(),
        ChannelManifestError::CompactIdCollision { compact_id: 1, .. }
    ));
}

#[test]
fn m5_receiver_layout_must_match_bound_entry() {
    let m = manifest();
    assert_eq!(m.verify_layout(1, &depth()).unwrap().name, "depth");
    // Same length, different wire identity: no length-only acceptance.
    assert_eq!(
        m.verify_layout(2, &depth()).unwrap_err(),
        ChannelManifestError::LayoutMismatch {
            name: "range".into()
        }
    );
    assert_eq!(
        m.verify_layout(3, &depth()).unwrap_err(),
        ChannelManifestError::UnknownCompactId(3)
    );
}

#[test]
fn m6_digest_is_pinned() {
    // Hash input: compact JSON {"manifest_version":..,"channels":[entries by compact_id]}.
    let m = ChannelManifest::new(vec![ChannelEntry::for_layout(1, "depth", &depth())]).unwrap();
    let e = &m.channels()[0];
    let input = format!(
        "{{\"manifest_version\":\"{CHANNEL_MANIFEST_VERSION}\",\"channels\":[{{\"compact_id\":1,\"name\":\"depth\",\"codec_id\":\"{}\",\"schema_id\":\"{}\",\"wire_id\":\"{}\",\"wire_len\":16}}]}}",
        e.codec_id, e.schema_id, e.wire_id
    );
    use sha2::Digest;
    let expected: [u8; 32] = sha2::Sha256::digest(input.as_bytes()).into();
    assert_eq!(m.digest_bytes(), expected);
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
