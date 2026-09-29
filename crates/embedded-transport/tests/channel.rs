//! WP-A02 compact channel envelope: collision-free tables and fail-closed resolution.
use neuradix_embedded_transport::channel::{ENVELOPE_MAGIC, ENVELOPE_VERSION, RESERVED_COMPACT_ID};
use neuradix_embedded_transport::{
    BindError, COMMAND_BYTES, ChannelBinding, ChannelTable, ENVELOPE_HEADER, EnvelopeError,
    FrameDecoder, FrameEvent, OVERHEAD, encode,
};

const DEPTH: &str = "sha256:4780f56bd7e4780987152907da8156c19dd2946470bc550c35abb16285d7b11d";
const OTHER: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const DIGEST: [u8; 32] = [0xA5; 32];

fn b(compact_id: u16, wire_id: &'static str, wire_len: u16) -> ChannelBinding {
    ChannelBinding {
        compact_id,
        wire_id,
        wire_len,
    }
}

fn table() -> ChannelTable<4> {
    ChannelTable::new(DIGEST, &[b(1, DEPTH, 16), b(2, OTHER, 16), b(3, OTHER, 4)]).unwrap()
}

#[test]
fn t1_round_trip_through_frame() {
    let t = table();
    let body: [u8; 16] = core::array::from_fn(|i| i as u8);
    let mut env = [0u8; ENVELOPE_HEADER + 16];
    assert_eq!(t.seal(1, &body, &mut env), Ok(env.len()));
    assert_eq!(&env[..4], &[ENVELOPE_MAGIC, ENVELOPE_VERSION, 1, 0]);
    assert_eq!(&env[4..8], &[0xA5; 4]);
    let mut wire = [0u8; OVERHEAD + ENVELOPE_HEADER + 16];
    let n = encode(9, &env, &mut wire).unwrap();
    let mut dec = FrameDecoder::<64>::new();
    let mut opened = None;
    for &byte in &wire[..n] {
        if let Some(FrameEvent::Frame(_)) = dec.push(byte) {
            let e = t.open(dec.payload()).unwrap();
            opened = Some((e.binding, <[u8; 16]>::try_from(e.body).unwrap()));
        }
    }
    let (binding, got) = opened.unwrap();
    assert_eq!(binding, b(1, DEPTH, 16));
    assert_eq!(got, body);
    assert_eq!(t.len(), 3);
    assert_eq!(t.manifest_digest(), &DIGEST);
}

#[test]
fn t2_collisions_and_bad_bindings_reject_the_whole_table() {
    let new = |bs: &[ChannelBinding]| ChannelTable::<4>::new(DIGEST, bs).map(|_| ()).unwrap_err();
    assert_eq!(
        new(&[b(1, DEPTH, 16), b(1, OTHER, 16)]),
        BindError::Collision { compact_id: 1 }
    );
    assert_eq!(
        new(&[b(1, DEPTH, 16), b(1, DEPTH, 16)]),
        BindError::Collision { compact_id: 1 }
    );
    assert_eq!(
        new(&[b(RESERVED_COMPACT_ID, DEPTH, 16)]),
        BindError::ReservedId
    );
    for bad in [
        "",
        "sha256:4780",
        "SHA256:4780f56bd7e4780987152907da8156c19dd2946470bc550c35abb16285d7b11d",
        "sha256:4780F56BD7E4780987152907DA8156C19DD2946470BC550C35ABB16285D7B11D",
        "sha256:4780f56bd7e4780987152907da8156c19dd2946470bc550c35abb16285d7b11g",
        "sha256:4c9c5d9381658f7779ef0d3ef11eda3f29006f7e751d06dba40a12d6f4ce2a730",
        "legacy-unversioned",
    ] {
        assert_eq!(
            new(&[b(4, bad, 16)]),
            BindError::MalformedWireId { compact_id: 4 },
            "{bad}"
        );
    }
    assert_eq!(
        new(&[b(4, DEPTH, 0)]),
        BindError::InvalidWireLen { compact_id: 4 }
    );
    assert_eq!(
        new(&[b(4, DEPTH, u16::MAX - ENVELOPE_HEADER as u16 + 1)]),
        BindError::InvalidWireLen { compact_id: 4 }
    );
    assert_eq!(new(&[]), BindError::Empty);
    let five: Vec<_> = (1..=5).map(|i| b(i, DEPTH, 16)).collect();
    assert_eq!(new(&five), BindError::TableFull);
    // A collision after valid bindings still yields no table.
    assert_eq!(
        new(&[b(1, DEPTH, 16), b(2, OTHER, 4), b(1, OTHER, 4)]),
        BindError::Collision { compact_id: 1 }
    );
}

fn env(tag: [u8; 4], id: u16, body: &[u8]) -> Vec<u8> {
    let mut v = vec![ENVELOPE_MAGIC, ENVELOPE_VERSION];
    v.extend_from_slice(&id.to_le_bytes());
    v.extend_from_slice(&tag);
    v.extend_from_slice(body);
    v
}

#[test]
fn t3_open_fails_closed() {
    let t = table();
    let tag = t.manifest_tag();
    assert_eq!(t.open(&[]).unwrap_err(), EnvelopeError::Truncated);
    assert_eq!(
        t.open(&env(tag, 1, &[])[..7]).unwrap_err(),
        EnvelopeError::Truncated
    );
    // A raw legacy 16-byte scalar payload without an envelope.
    assert_eq!(t.open(&[0x42; 16]).unwrap_err(), EnvelopeError::BadMagic);
    let mut v = env(tag, 1, &[0; 16]);
    v[1] = 2;
    assert_eq!(
        t.open(&v).unwrap_err(),
        EnvelopeError::UnsupportedVersion(2)
    );
    // Peer provisioned from a different manifest.
    assert_eq!(
        t.open(&env([0xA5, 0xA5, 0xA5, 0xA4], 1, &[0; 16]))
            .unwrap_err(),
        EnvelopeError::ManifestMismatch
    );
    assert_eq!(
        t.open(&env(tag, 9, &[0; 16])).unwrap_err(),
        EnvelopeError::UnknownChannel(9)
    );
    assert_eq!(
        t.open(&env(tag, 0, &[0; 16])).unwrap_err(),
        EnvelopeError::UnknownChannel(0)
    );
    for len in [0, 15, 17] {
        assert_eq!(
            t.open(&env(tag, 1, &vec![0; len])).unwrap_err(),
            EnvelopeError::LengthMismatch {
                compact_id: 1,
                expected: 16,
                actual: len
            }
        );
    }
    // Channel 3 is a 4-byte OTHER; a 16-byte body is not reinterpreted.
    assert!(matches!(
        t.open(&env(tag, 3, &[0; 16])),
        Err(EnvelopeError::LengthMismatch { .. })
    ));
}

#[test]
fn t4_command_payloads_are_not_envelopes() {
    let t = table();
    let mut cmd = [0u8; COMMAND_BYTES];
    cmd[0] = 1; // COMMAND_VERSION
    cmd[1] = 1;
    assert_eq!(t.open(&cmd).unwrap_err(), EnvelopeError::BadMagic);
}

#[test]
fn t5_seal_checks_binding_and_buffer() {
    let t = table();
    let mut out = [0u8; 64];
    assert_eq!(
        t.seal(9, &[0; 16], &mut out),
        Err(EnvelopeError::UnknownChannel(9))
    );
    assert_eq!(
        t.seal(0, &[0; 16], &mut out),
        Err(EnvelopeError::UnknownChannel(0))
    );
    assert_eq!(
        t.seal(3, &[0; 16], &mut out),
        Err(EnvelopeError::LengthMismatch {
            compact_id: 3,
            expected: 4,
            actual: 16
        })
    );
    assert_eq!(
        t.seal(1, &[0; 16], &mut out[..23]),
        Err(EnvelopeError::BufferTooSmall)
    );
    assert_eq!(t.seal(3, &[1, 2, 3, 4], &mut out), Ok(12));
    assert_eq!(t.open(&out[..12]).unwrap().body, &[1, 2, 3, 4]);
    // A table from another manifest rejects this table's envelopes.
    let other = ChannelTable::<4>::new([0x5A; 32], &[b(3, OTHER, 4)]).unwrap();
    assert_eq!(
        other.open(&out[..12]).unwrap_err(),
        EnvelopeError::ManifestMismatch
    );
}
