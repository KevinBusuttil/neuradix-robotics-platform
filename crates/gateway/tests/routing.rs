//! Runtime routing through the actual gateway: real framed bytes in, typed
//! handler calls out, and no handler call for any rejected frame.
mod common;

use std::path::Path;

use common::{Producer, Recorder, depth, gateway, producer, tiny};
use neuradix_contracts::ChannelManifest;
use neuradix_embedded_transport::{EnvelopeError, SeqStatus};
use neuradix_gateway::reference::{self, TINY_TELEMETRY_ID, TinyTelemetry, VEHICLE_DEPTH_ID};
use neuradix_gateway::{Delivered, FrameOutcome, IdentityClaims, IdentitySource, Rejection};

fn outcomes(chunks: &[&[u8]]) -> (Vec<FrameOutcome>, Recorder, neuradix_gateway::Stats) {
    let mut g = gateway();
    let mut r = Recorder::default();
    let mut out = Vec::new();
    for c in chunks {
        g.push_observed(c, &mut r, |o| out.push(o));
    }
    (out, r, *g.stats())
}

fn rejected(o: &FrameOutcome) -> Option<Rejection> {
    match o {
        FrameOutcome::Rejected { reason, .. } => Some(*reason),
        FrameOutcome::Delivered { .. } => None,
    }
}

#[test]
fn r1_both_routes_dispatch_typed_values() {
    let mut p = producer();
    let a = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(12.5));
    let b = p.tiny_telemetry(TINY_TELEMETRY_ID, &tiny(7));
    let (out, r, s) = outcomes(&[&a, &b]);
    assert_eq!(r.depth, [(1, depth(12.5))]);
    assert_eq!(r.tiny, [(2, tiny(7))]);
    assert_eq!(
        out,
        [
            FrameOutcome::Delivered {
                seq: 0,
                sequence: SeqStatus::First,
                channel: Delivered {
                    compact_id: 1,
                    name: "vehicle-depth"
                }
            },
            FrameOutcome::Delivered {
                seq: 1,
                sequence: SeqStatus::InOrder,
                channel: Delivered {
                    compact_id: 2,
                    name: "tiny-telemetry"
                }
            },
        ]
    );
    assert_eq!((s.frames, s.delivered, s.rejected()), (2, 2, 0));
    assert_eq!(s.bytes, (a.len() + b.len()) as u64);
}

#[test]
fn r2_fragmented_input_decodes_at_every_split_point() {
    let mut p = producer();
    let stream = [
        p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(1.0)),
        p.tiny_telemetry(TINY_TELEMETRY_ID, &tiny(2)),
    ]
    .concat();
    for split in 0..=stream.len() {
        let (_, r, s) = outcomes(&[&stream[..split], &stream[split..]]);
        assert_eq!((r.depth.len(), r.tiny.len()), (1, 1), "split {split}");
        assert_eq!(s.rejected(), 0);
    }
    // One byte at a time, and irregular chunk sizes.
    let bytes: Vec<&[u8]> = stream.chunks(1).collect();
    let (_, r, _) = outcomes(&bytes);
    assert_eq!(r.calls(), 2);
    for size in [2, 3, 5, 7, 11, 13] {
        let chunks: Vec<&[u8]> = stream.chunks(size).collect();
        let (_, r, _) = outcomes(&chunks);
        assert_eq!(r.depth, [(1, depth(1.0))], "chunk size {size}");
        assert_eq!(r.tiny, [(2, tiny(2))], "chunk size {size}");
    }
}

#[test]
fn r3_many_frames_in_one_chunk_are_all_routed_in_order() {
    let mut p = producer();
    let mut stream = Vec::new();
    for i in 0..50u32 {
        if i % 2 == 0 {
            stream.extend(p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(f64::from(i))));
        } else {
            stream.extend(p.tiny_telemetry(TINY_TELEMETRY_ID, &tiny(i)));
        }
    }
    let mut g = gateway();
    let mut r = Recorder::default();
    let summary = g.push(&stream, &mut r);
    assert_eq!((summary.delivered, summary.rejected), (50, 0));
    let depths: Vec<f64> = r.depth.iter().map(|(_, v)| v.depth).collect();
    assert_eq!(
        depths,
        (0..50).step_by(2).map(f64::from).collect::<Vec<_>>()
    );
    let tinies: Vec<u32> = r.tiny.iter().map(|(_, v)| v.e_unsigned32).collect();
    assert_eq!(tinies, (1..50).step_by(2).collect::<Vec<_>>());
}

#[test]
fn r4_crc_failures_reach_no_handler() {
    let mut p = producer();
    let good = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(3.0));
    let next = p.tiny_telemetry(TINY_TELEMETRY_ID, &tiny(3));
    // Flip each byte after the sync pattern (seq, len, envelope, body, CRC).
    for i in 2..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0x01;
        let mut g = gateway();
        let mut r = Recorder::default();
        let summary = g.push(&bad, &mut r);
        assert_eq!(summary.delivered, 0, "byte {i}");
        assert_eq!(r.calls(), 0, "byte {i}");
        // Flipping the length can leave the decoder waiting for more bytes;
        // otherwise the CRC fails outright.
        g.discard_partial();
        g.push(&next, &mut r);
        assert_eq!(r.tiny, [(2, tiny(3))], "byte {i}");
    }
    let mut bad = good.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x80;
    let (out, r, s) = outcomes(&[&bad, &next]);
    assert_eq!(rejected(&out[0]), Some(Rejection::CorruptFrame));
    assert_eq!((r.calls(), s.corrupt, s.delivered), (1, 1, 1));
}

#[test]
fn r5_oversized_frames_are_dropped_without_buffering() {
    let mut p = producer();
    // 65 > 64-byte frame buffer; and the largest declarable length.
    let over = p.frame(&[0x11; reference::FRAME_CAPACITY + 1]);
    let mut huge = p.frame(&[]);
    huge[4..6].copy_from_slice(&u16::MAX.to_le_bytes());
    let valid = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(5.0));
    let (out, r, s) = outcomes(&[&over, &huge[..6], &valid]);
    assert_eq!(rejected(&out[0]), Some(Rejection::CorruptFrame));
    assert_eq!(rejected(&out[1]), Some(Rejection::CorruptFrame));
    assert_eq!(r.depth, [(1, depth(5.0))]);
    assert_eq!((s.corrupt, s.delivered), (2, 1));
    // A frame that fits the buffer but not any bound channel is refused by the
    // envelope, not delivered.
    let tag = p.manifest_tag();
    let fits = p.frame(&Producer::raw_envelope(
        1,
        tag,
        &[0; reference::FRAME_CAPACITY - 8],
    ));
    let (out, r, _) = outcomes(&[&fits]);
    assert_eq!(
        rejected(&out[0]),
        Some(Rejection::Envelope(EnvelopeError::LengthMismatch {
            compact_id: 1,
            expected: 16,
            actual: 56
        }))
    );
    assert_eq!(r.calls(), 0);
}

#[test]
fn r6_incomplete_frames_deliver_nothing() {
    let mut p = producer();
    let partial = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(6.0));
    let v = p.tiny_telemetry(TINY_TELEMETRY_ID, &tiny(6));
    let w = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(7.0));

    // A partial frame alone: nothing delivered, nothing counted yet.
    let mut g = gateway();
    let mut r = Recorder::default();
    g.push(&partial[..10], &mut r);
    assert_eq!(
        (r.calls(), g.stats().frames, g.stats().rejected()),
        (0, 0, 0)
    );
    // Completing it later delivers it.
    g.push(&partial[10..], &mut r);
    assert_eq!(r.depth, [(1, depth(6.0))]);

    // Never completed: the next frame's bytes are read as its remainder, the
    // CRC fails and that frame is lost too; the one after is delivered.
    let (out, r, s) = outcomes(&[&partial[..10], &v, &w]);
    assert_eq!(rejected(&out[0]), Some(Rejection::CorruptFrame));
    assert_eq!(
        (r.tiny.len(), r.depth.as_slice()),
        (0, &[(1, depth(7.0))][..])
    );
    assert_eq!((s.corrupt, s.delivered), (1, 1));

    // Discarding the partial frame (for example after an idle gap) keeps V.
    let mut g = gateway();
    let mut r = Recorder::default();
    g.push(&partial[..10], &mut r);
    g.discard_partial();
    g.push(&[v.as_slice(), w.as_slice()].concat(), &mut r);
    assert_eq!((r.tiny.len(), r.depth.len(), g.stats().corrupt), (1, 1, 0));
}

#[test]
fn r7_unknown_reserved_short_long_and_malformed_envelopes_are_refused() {
    let mut p = producer();
    let tag = p.manifest_tag();
    let raw = Producer::raw_envelope;
    let mut v2 = raw(1, tag, &[0; 16]);
    v2[1] = 2;
    let cases: Vec<(Vec<u8>, EnvelopeError)> = vec![
        (raw(9, tag, &[0; 16]), EnvelopeError::UnknownChannel(9)),
        (raw(0, tag, &[0; 16]), EnvelopeError::UnknownChannel(0)),
        (
            raw(u16::MAX, tag, &[0; 29]),
            EnvelopeError::UnknownChannel(u16::MAX),
        ),
        (
            raw(1, tag, &[0; 15]),
            EnvelopeError::LengthMismatch {
                compact_id: 1,
                expected: 16,
                actual: 15,
            },
        ),
        (
            raw(1, tag, &[0; 17]),
            EnvelopeError::LengthMismatch {
                compact_id: 1,
                expected: 16,
                actual: 17,
            },
        ),
        // A valid vehicle-depth body on the tiny-telemetry channel.
        (
            raw(2, tag, &[0; 16]),
            EnvelopeError::LengthMismatch {
                compact_id: 2,
                expected: 29,
                actual: 16,
            },
        ),
        (raw(1, tag, &[])[..7].to_vec(), EnvelopeError::Truncated),
        (vec![0; 24], EnvelopeError::BadMagic),
        (v2, EnvelopeError::UnsupportedVersion(2)),
    ];
    for (payload, expected) in cases {
        let frame = p.frame(&payload);
        let (out, r, s) = outcomes(&[&frame]);
        assert_eq!(rejected(&out[0]), Some(Rejection::Envelope(expected)));
        assert_eq!((r.calls(), s.frames, s.rejected()), (0, 1, 1));
    }
}

#[test]
fn r8_equal_width_foreign_contract_never_reaches_the_depth_handler() {
    let mut p = producer();
    // Range bytes from a producer provisioned with the foreign manifest (range
    // on compact ID 1, valid digest): refused by the manifest tag.
    let foreign = ChannelManifest::parse(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/foreign-manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let foreign_tag: [u8; 4] = foreign.digest_bytes()[..4].try_into().unwrap();
    assert_ne!(foreign_tag, p.manifest_tag());
    let range_body = [0x3f; 16];
    let frame = p.frame(&Producer::raw_envelope(1, foreign_tag, &range_body));
    let (out, r, s) = outcomes(&[&frame]);
    assert_eq!(
        rejected(&out[0]),
        Some(Rejection::Envelope(EnvelopeError::ManifestMismatch))
    );
    assert_eq!((r.calls(), s.manifest_mismatch), (0, 1));
    // And the gateway cannot be configured with that manifest at all.
    assert!(reference::gateway::<Recorder>(&foreign.to_json_pretty()).is_err());
}

/// The limit of this increment, stated as a test: the four-byte tag is not
/// authentication. A sender that copies the reference tag and puts 16 foreign
/// bytes on channel 1 is decoded as `VehicleDepth`. Only a link session with
/// full manifest exchange and authentication (WP-B06) can refuse it.
#[test]
fn r9_a_copied_manifest_tag_is_not_authentication() {
    let mut p = producer();
    let tag = p.manifest_tag();
    let frame = p.frame(&Producer::raw_envelope(1, tag, &[0x3f; 16]));
    let (_, r, _) = outcomes(&[&frame]);
    assert_eq!(r.depth.len(), 1);
}

#[test]
fn r10_decoder_rejection_calls_no_handler() {
    let mut p = producer();
    let mut body = [0; TinyTelemetry::WIRE_LEN];
    tiny(4).encode(&mut body);
    body[0] = 2; // non-canonical bool
    let tag = p.manifest_tag();
    let frame = p.frame(&Producer::raw_envelope(2, tag, &body));
    let (out, r, s) = outcomes(&[&frame]);
    assert_eq!(rejected(&out[0]), Some(Rejection::Decode { compact_id: 2 }));
    assert_eq!((r.calls(), s.decode_rejected), (0, 1));
}

#[test]
fn r11_sequence_numbers_are_observed_but_not_enforced() {
    let mut p = producer();
    let a = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(1.0));
    let _lost = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(2.0));
    let c = p.vehicle_depth(VEHICLE_DEPTH_ID, &depth(3.0));
    // Duplicate, gap and reorder are all delivered: replay protection and
    // duplicate suppression are WP-B06 session work.
    let (out, r, s) = outcomes(&[&a, &a, &c, &a]);
    let seqs: Vec<_> = out
        .iter()
        .map(|o| match o {
            FrameOutcome::Delivered { sequence, .. } => *sequence,
            FrameOutcome::Rejected { .. } => panic!("unexpected rejection"),
        })
        .collect();
    assert_eq!(
        seqs,
        [
            SeqStatus::First,
            SeqStatus::Duplicate,
            SeqStatus::Gap(1),
            SeqStatus::Reordered
        ]
    );
    assert_eq!(r.depth.len(), 4);
    assert_eq!((s.duplicates, s.missed, s.reordered), (1, 1, 1));
}

#[test]
fn r12_identity_report_states_digest_and_sources() {
    let g = gateway();
    let report = g.identity(reference::simulated_claims());
    assert_eq!(
        report.manifest_digest,
        "sha256:4a8cf8f6ba9d3479fad842b80b177e0d90b93bcf259f070fd37e4ac9a00d97fb"
    );
    assert_eq!(
        report.claims.node.as_ref().unwrap().source,
        IdentitySource::SimulatedFixture
    );
    assert_eq!(
        report.claims.firmware.as_ref().unwrap().source,
        IdentitySource::SimulatedFixture
    );
    assert_eq!(
        report.claims.deployment.as_ref().unwrap().source,
        IdentitySource::SimulatedFixture
    );
    assert_eq!(report.gateway_build.source, IdentitySource::GeneratedBuild);
    let text = report.to_string();
    assert!(text.contains(&report.manifest_digest));
    assert!(text.contains("[simulated fixture]"));
    assert!(text.contains("no peer attestation"));
    // Absent identities are reported as absent, not invented.
    let bare = g.identity(IdentityClaims::default()).to_string();
    assert_eq!(bare.matches("(not configured)").count(), 3);
}
