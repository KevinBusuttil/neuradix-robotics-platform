//! Round-trip, determinism and corruption tests for the native recording format.

use neuradix_record::{
    Channel, NativeRecordWriter, NativeRecording, RecordCodec, RecordError, RecordingManifest,
    SoftwareId, replay_digest,
};
use neuradix_time::{ClockDomain, Timestamp};

/// A trivial fixed-layout codec for a `(u32, i32)` message.
struct PairCodec;

impl RecordCodec for PairCodec {
    type Message = (u32, i32);

    fn encode(&self, m: &Self::Message) -> Vec<u8> {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(&m.0.to_le_bytes());
        out.extend_from_slice(&m.1.to_le_bytes());
        out
    }

    fn decode(&self, bytes: &[u8]) -> Result<Self::Message, RecordError> {
        if bytes.len() != 8 {
            return Err(RecordError::Decode(format!(
                "expected 8 bytes, got {}",
                bytes.len()
            )));
        }
        let a = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let b = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        Ok((a, b))
    }
}

fn sample_manifest() -> RecordingManifest {
    RecordingManifest::builder("neuradix-record/test")
        .channel(Channel {
            id: 0,
            name: "test/pair".to_owned(),
            schema_id: "sha256:deadbeef".to_owned(),
            clock_domain: "simulation".to_owned(),
        })
        .software(SoftwareId::new("test", "0.0.1"))
        .seed(42)
        .note("roundtrip")
        .build()
}

fn record_pairs(pairs: &[(u32, i32)]) -> Vec<u8> {
    let codec = PairCodec;
    let mut writer = NativeRecordWriter::new(Vec::new(), &sample_manifest()).unwrap();
    for (i, pair) in pairs.iter().enumerate() {
        let ts = Timestamp::new(ClockDomain::Simulation, i as i128 * 1_000_000);
        writer
            .write_record(0, i as u64, ts, &codec.encode(pair))
            .unwrap();
    }
    writer.finish().unwrap()
}

#[test]
fn records_round_trip_byte_for_byte() {
    let pairs = [(1u32, -1i32), (2, -2), (3, 3), (1000, 12345)];
    let bytes = record_pairs(&pairs);

    let recording = NativeRecording::from_bytes(&bytes).unwrap();
    assert_eq!(recording.manifest().writer, "neuradix-record/test");
    assert_eq!(recording.manifest().seed, Some(42));
    assert_eq!(recording.count_for(0), pairs.len());

    let codec = PairCodec;
    let decoded: Vec<(u32, i32)> = recording
        .records_for(0)
        .map(|r| codec.decode(&r.payload).unwrap())
        .collect();
    assert_eq!(decoded, pairs);

    // Timestamps and domains survive the round trip.
    for (i, record) in recording.records().iter().enumerate() {
        assert_eq!(record.sequence, i as u64);
        assert_eq!(record.timestamp.domain(), ClockDomain::Simulation);
        assert_eq!(record.timestamp.as_nanos(), i as i128 * 1_000_000);
    }
}

#[test]
fn replay_digest_is_deterministic_and_sensitive() {
    let a = NativeRecording::from_bytes(&record_pairs(&[(1, 2), (3, 4)])).unwrap();
    let b = NativeRecording::from_bytes(&record_pairs(&[(1, 2), (3, 4)])).unwrap();
    assert_eq!(
        replay_digest(&a),
        replay_digest(&b),
        "same records => same digest"
    );
    assert!(replay_digest(&a).starts_with("sha256:"));

    let different = NativeRecording::from_bytes(&record_pairs(&[(1, 2), (3, 5)])).unwrap();
    assert_ne!(
        replay_digest(&a),
        replay_digest(&different),
        "changed payload => changed digest"
    );
}

#[test]
fn bad_magic_is_rejected() {
    let err = NativeRecording::from_bytes(b"not a recording at all").unwrap_err();
    assert!(matches!(err, RecordError::BadMagic));
}

#[test]
fn truncated_recording_is_rejected() {
    let bytes = record_pairs(&[(1, 2), (3, 4)]);
    // Cut off mid-record.
    let err = NativeRecording::from_bytes(&bytes[..bytes.len() - 3]).unwrap_err();
    assert!(matches!(err, RecordError::Truncated(_)), "got {err:?}");
}

#[test]
fn empty_recording_has_manifest_and_no_records() {
    let bytes = record_pairs(&[]);
    let recording = NativeRecording::from_bytes(&bytes).unwrap();
    assert_eq!(recording.records().len(), 0);
    assert_eq!(recording.manifest().channels.len(), 1);
    assert!(replay_digest(&recording).starts_with("sha256:"));
}
