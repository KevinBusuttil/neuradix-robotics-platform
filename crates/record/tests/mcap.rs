//! MCAP backend tests: round-trip fidelity, cross-container replay equivalence,
//! byte-structure conformance and corruption handling.

use neuradix_record::{
    Channel, MCAP_MAGIC, McapRecording, McapWriter, NativeRecordWriter, NativeRecording,
    RecordError, Recording, RecordingManifest, SoftwareId, replay_digest,
};
use neuradix_time::{ClockDomain, Timestamp};

fn manifest() -> RecordingManifest {
    RecordingManifest::builder("neuradix-record/test")
        .channel(Channel {
            id: 0,
            name: "navigation/vehicle-depth".to_owned(),
            schema_id: "sha256:deadbeef".to_owned(),
            clock_domain: "simulation".to_owned(),
        })
        .channel(Channel {
            id: 1,
            name: "actuation/thrust".to_owned(),
            schema_id: "sha256:c0ffee".to_owned(),
            clock_domain: "monotonic".to_owned(),
        })
        .software(SoftwareId::new("test", "0.0.1"))
        .seed(7)
        .note("mcap-roundtrip")
        .build()
}

/// Write a two-channel recording with the given payloads to MCAP bytes.
fn write_mcap(samples: &[(u16, u64, i128, &[u8])]) -> Vec<u8> {
    let mut w = McapWriter::new(Vec::new(), &manifest()).unwrap();
    for &(channel, seq, nanos, payload) in samples {
        let domain = if channel == 1 {
            ClockDomain::Monotonic
        } else {
            ClockDomain::Simulation
        };
        w.write_record(channel, seq, Timestamp::new(domain, nanos), payload)
            .unwrap();
    }
    w.finish().unwrap()
}

fn samples() -> Vec<(u16, u64, i128, &'static [u8])> {
    vec![
        (0, 0, 0, b"depth-0".as_slice()),
        (0, 1, 1_000_000, b"depth-1".as_slice()),
        (1, 0, 1_500_000, b"thr".as_slice()),
        (0, 2, 2_000_000, b"depth-2".as_slice()),
    ]
}

#[test]
fn mcap_starts_and_ends_with_magic() {
    let bytes = write_mcap(&samples());
    assert_eq!(&bytes[..8], &MCAP_MAGIC);
    assert_eq!(&bytes[bytes.len() - 8..], &MCAP_MAGIC);
    // First record after the magic is the Header (opcode 0x01).
    assert_eq!(bytes[8], 0x01);
}

#[test]
fn mcap_round_trips_records_and_manifest() {
    let bytes = write_mcap(&samples());
    let rec = McapRecording::from_bytes(&bytes).unwrap();

    // Manifest is preserved losslessly via the embedded metadata record.
    assert_eq!(rec.manifest().writer, "neuradix-record/test");
    assert_eq!(rec.manifest().seed, Some(7));
    assert_eq!(rec.manifest().note.as_deref(), Some("mcap-roundtrip"));
    assert_eq!(rec.manifest().channels.len(), 2);
    assert_eq!(rec.manifest().software.len(), 1);

    // Records are preserved in order, with domain, timestamp and payload intact.
    let expect = samples();
    assert_eq!(rec.records().len(), expect.len());
    for (got, (ch, seq, nanos, payload)) in rec.records().iter().zip(expect) {
        assert_eq!(got.channel_id, ch);
        assert_eq!(got.sequence, seq);
        assert_eq!(got.timestamp.as_nanos(), nanos);
        assert_eq!(got.payload, payload);
        let expected_domain = if ch == 1 {
            ClockDomain::Monotonic
        } else {
            ClockDomain::Simulation
        };
        assert_eq!(got.timestamp.domain(), expected_domain);
    }
    assert_eq!(rec.count_for(0), 3);
    assert_eq!(rec.count_for(1), 1);
}

#[test]
fn native_and_mcap_agree_on_replay_digest() {
    // The same records, written to each container, must digest identically —
    // cross-container replay equivalence.
    let mut native = NativeRecordWriter::new(Vec::new(), &manifest()).unwrap();
    for &(channel, seq, nanos, payload) in &samples() {
        let domain = if channel == 1 {
            ClockDomain::Monotonic
        } else {
            ClockDomain::Simulation
        };
        native
            .write_record(channel, seq, Timestamp::new(domain, nanos), payload)
            .unwrap();
    }
    let native_bytes = native.finish().unwrap();

    let native_rec = NativeRecording::from_bytes(&native_bytes).unwrap();
    let mcap_rec = McapRecording::from_bytes(&write_mcap(&samples())).unwrap();

    assert_eq!(
        replay_digest(&native_rec),
        replay_digest(&mcap_rec),
        "native and MCAP recordings of the same records must share a replay digest"
    );
}

#[test]
fn mcap_output_is_deterministic() {
    assert_eq!(
        write_mcap(&samples()),
        write_mcap(&samples()),
        "MCAP encoding must be byte-stable"
    );
}

#[test]
fn empty_mcap_round_trips() {
    let bytes = write_mcap(&[]);
    assert_eq!(&bytes[..8], &MCAP_MAGIC);
    let rec = McapRecording::from_bytes(&bytes).unwrap();
    assert_eq!(rec.records().len(), 0);
    // The two manifest channels survive even with no messages.
    assert_eq!(rec.manifest().channels.len(), 2);
    assert!(replay_digest(&rec).starts_with("sha256:"));
}

#[test]
fn bad_magic_is_rejected() {
    let err = McapRecording::from_bytes(b"definitely not an mcap file!!!!!").unwrap_err();
    assert!(matches!(err, RecordError::Mcap(_)), "got {err:?}");
}

#[test]
fn truncated_mcap_is_rejected() {
    let bytes = write_mcap(&samples());
    let err = McapRecording::from_bytes(&bytes[..bytes.len() - 12]).unwrap_err();
    assert!(
        matches!(err, RecordError::Mcap(_) | RecordError::Truncated(_)),
        "got {err:?}"
    );
}

#[test]
fn a_negative_timestamp_is_rejected() {
    let mut w = McapWriter::new(Vec::new(), &manifest()).unwrap();
    let err = w
        .write_record(0, 0, Timestamp::new(ClockDomain::Simulation, -1), b"x")
        .unwrap_err();
    assert!(
        matches!(err, RecordError::TimestampOutOfRange(-1)),
        "got {err:?}"
    );
}

#[test]
fn a_sequence_beyond_u32_is_rejected() {
    let mut w = McapWriter::new(Vec::new(), &manifest()).unwrap();
    let seq = u64::from(u32::MAX) + 1;
    let err = w
        .write_record(0, seq, Timestamp::new(ClockDomain::Simulation, 0), b"x")
        .unwrap_err();
    assert!(
        matches!(err, RecordError::SequenceTooLarge(_)),
        "got {err:?}"
    );
}

/// Locate the summary `Statistics` record (opcode 0x0B) and return its content,
/// by walking the `opcode(u8) + length(u64 LE) + content` record framing.
fn find_statistics(bytes: &[u8]) -> Vec<u8> {
    let mut pos = 8; // after leading magic
    let end = bytes.len() - 8; // before trailing magic
    while pos < end {
        let opcode = bytes[pos];
        pos += 1;
        let len = u64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        let content = &bytes[pos..pos + len];
        pos += len;
        if opcode == 0x0B {
            return content.to_vec();
        }
    }
    panic!("no Statistics record found");
}

#[test]
fn statistics_reports_message_and_metadata_counts() {
    let bytes = write_mcap(&samples());
    let stats = find_statistics(&bytes);
    // Layout: message_count(u64) schema_count(u16) channel_count(u32)
    //         attachment_count(u32) metadata_count(u32) ...
    let message_count = u64::from_le_bytes(stats[0..8].try_into().unwrap());
    let metadata_count = u32::from_le_bytes(stats[18..22].try_into().unwrap());
    assert_eq!(message_count, samples().len() as u64);
    // Exactly one Metadata record (the embedded manifest) is written.
    assert_eq!(
        metadata_count, 1,
        "statistics must count the manifest metadata record"
    );
}

#[test]
fn too_many_channels_is_rejected() {
    // The MCAP schema-id space is u16 (0 reserved), so at most 65535 channels
    // can be represented. A manifest with all 65536 channel ids must fail
    // cleanly rather than overflow the schema-id counter.
    let mut builder = RecordingManifest::builder("neuradix-record/test");
    for id in 0..=u16::MAX {
        builder = builder.channel(Channel {
            id,
            name: format!("c{id}"),
            schema_id: String::new(),
            clock_domain: "monotonic".to_owned(),
        });
    }
    let err = McapWriter::new(Vec::new(), &builder.build())
        .unwrap()
        .finish()
        .unwrap_err();
    assert!(
        matches!(err, RecordError::TooManyChannels(_)),
        "got {err:?}"
    );
}
