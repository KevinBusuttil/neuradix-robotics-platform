//! Behavioural tests for the Studio inspection model.

use neuradix_record::{Channel, NativeRecordWriter, NativeRecording, RecordingManifest};
use neuradix_studio::{Inspection, ScalarDecoder, ScalarSample, StudioError};
use neuradix_time::{ClockDomain, Timestamp};

/// A decoder for a fixed-layout `[f64 depth][f64 thrust]` payload.
struct PairDecoder;

impl ScalarDecoder for PairDecoder {
    fn decode(&self, payload: &[u8]) -> Result<Vec<ScalarSample>, StudioError> {
        if payload.len() != 16 {
            return Err(StudioError::Decode(format!(
                "want 16 bytes, got {}",
                payload.len()
            )));
        }
        let depth = f64::from_le_bytes(payload[0..8].try_into().unwrap());
        let thrust = f64::from_le_bytes(payload[8..16].try_into().unwrap());
        Ok(vec![
            ScalarSample::new("depth", depth),
            ScalarSample::new("thrust", thrust),
        ])
    }
}

fn pair(depth: f64, thrust: f64) -> Vec<u8> {
    let mut v = Vec::with_capacity(16);
    v.extend_from_slice(&depth.to_le_bytes());
    v.extend_from_slice(&thrust.to_le_bytes());
    v
}

fn manifest() -> RecordingManifest {
    RecordingManifest::builder("neuradix-studio/test")
        .channel(Channel {
            id: 0,
            name: "navigation/depth".to_owned(),
            schema_id: "sha256:depth".to_owned(),
            clock_domain: "simulation".to_owned(),
        })
        .channel(Channel {
            id: 9,
            name: "unused/channel".to_owned(),
            schema_id: "sha256:none".to_owned(),
            clock_domain: "monotonic".to_owned(),
        })
        .build()
}

/// A recording with five depth samples on channel 0 at 20 ms spacing (50 Hz).
fn recording() -> NativeRecording {
    let mut w = NativeRecordWriter::new(Vec::new(), &manifest()).unwrap();
    for i in 0..5u64 {
        let ts = Timestamp::new(ClockDomain::Simulation, i as i128 * 20_000_000);
        w.write_record(0, i, ts, &pair(i as f64, i as f64 * 0.1))
            .unwrap();
    }
    let bytes = w.finish().unwrap();
    NativeRecording::from_bytes(&bytes).unwrap()
}

#[test]
fn timeline_summarizes_channels_and_rate() {
    let rec = recording();
    let studio = Inspection::new(&rec);
    let tl = studio.timeline();

    assert_eq!(tl.message_count, 5);
    assert_eq!(tl.channel_count, 2); // both manifest channels are described

    let depth = tl.channels.iter().find(|c| c.id == 0).unwrap();
    assert_eq!(depth.count, 5);
    assert_eq!(depth.first_nanos, Some(0));
    assert_eq!(depth.last_nanos, Some(80_000_000));
    assert_eq!(depth.span_nanos, Some(80_000_000));
    assert_eq!(depth.mean_period_nanos, Some(20_000_000));
    assert!((depth.rate_hz.unwrap() - 50.0).abs() < 1e-6);
    assert_eq!(depth.min_payload, Some(16));
    assert_eq!(depth.max_payload, Some(16));
    assert_eq!(depth.total_payload, 80);
    assert_eq!(depth.clock_domain, "simulation");

    // The manifest channel with no records still appears, empty.
    let unused = tl.channels.iter().find(|c| c.id == 9).unwrap();
    assert_eq!(unused.count, 0);
    assert_eq!(unused.rate_hz, None);
    assert_eq!(unused.first_nanos, None);
    assert_eq!(unused.clock_domain, "monotonic"); // from the manifest hint
}

#[test]
fn timeline_reports_one_domain_span() {
    let rec = recording();
    let tl = Inspection::new(&rec).timeline();
    assert_eq!(tl.domains.len(), 1);
    let span = &tl.domains[0];
    assert_eq!(span.domain, "simulation");
    assert_eq!(span.start_nanos, 0);
    assert_eq!(span.end_nanos, 80_000_000);
    assert_eq!(span.duration_nanos, 80_000_000);
    assert_eq!(span.message_count, 5);
}

#[test]
fn multiple_domains_are_reported_separately() {
    let mut w = NativeRecordWriter::new(Vec::new(), &manifest()).unwrap();
    w.write_record(
        0,
        0,
        Timestamp::new(ClockDomain::Simulation, 0),
        &pair(1.0, 0.0),
    )
    .unwrap();
    w.write_record(
        0,
        1,
        Timestamp::new(ClockDomain::Simulation, 10_000_000),
        &pair(2.0, 0.0),
    )
    .unwrap();
    // A record in a different domain on another channel.
    w.write_record(
        9,
        0,
        Timestamp::new(ClockDomain::Monotonic, 5_000_000),
        &pair(0.0, 0.0),
    )
    .unwrap();
    let bytes = w.finish().unwrap();
    let rec = NativeRecording::from_bytes(&bytes).unwrap();

    let tl = Inspection::new(&rec).timeline();
    assert_eq!(tl.domains.len(), 2);
    let sim = tl
        .domains
        .iter()
        .find(|d| d.domain == "simulation")
        .unwrap();
    let mono = tl.domains.iter().find(|d| d.domain == "monotonic").unwrap();
    assert_eq!(sim.message_count, 2);
    assert_eq!(mono.message_count, 1);
}

#[test]
fn window_is_inclusive_and_ordered() {
    let rec = recording();
    let studio = Inspection::new(&rec);

    // [20ms, 60ms] inclusive -> samples at 20, 40, 60 ms.
    let w = studio.window(0, 20_000_000, 60_000_000);
    assert_eq!(w.len(), 3);
    assert_eq!(w[0].timestamp.as_nanos(), 20_000_000);
    assert_eq!(w[2].timestamp.as_nanos(), 60_000_000);

    // Empty range.
    assert!(studio.window(0, 1, 2).is_empty());
    // Unknown channel.
    assert!(studio.window(42, 0, i128::MAX).is_empty());
    // Whole range.
    assert_eq!(studio.window(0, i128::MIN, i128::MAX).len(), 5);
}

#[test]
fn nearest_picks_the_closest_and_ties_to_earlier() {
    let rec = recording();
    let studio = Inspection::new(&rec);

    assert_eq!(studio.nearest(0, 0).unwrap().timestamp.as_nanos(), 0);
    // 33 ms is closer to 40 ms than 20 ms.
    assert_eq!(
        studio.nearest(0, 33_000_000).unwrap().timestamp.as_nanos(),
        40_000_000
    );
    // Exactly between 20 and 40 ms -> tie resolves to the earlier (20 ms).
    assert_eq!(
        studio.nearest(0, 30_000_000).unwrap().timestamp.as_nanos(),
        20_000_000
    );
    // Beyond the end clamps to the last.
    assert_eq!(
        studio.nearest(0, 999_000_000).unwrap().timestamp.as_nanos(),
        80_000_000
    );
    // Unknown / empty channel.
    assert!(studio.nearest(9, 0).is_none());
    assert!(studio.nearest(42, 0).is_none());
}

#[test]
fn series_extracts_a_field_with_stats() {
    let rec = recording();
    let studio = Inspection::new(&rec);

    let s = studio.series(0, "depth", &PairDecoder).unwrap();
    assert_eq!(s.field, "depth");
    assert_eq!(s.domain, "simulation");
    assert_eq!(s.points.len(), 5);
    assert_eq!(s.points[0].nanos, 0);
    assert_eq!(s.points[0].value, 0.0);
    assert_eq!(s.points[4].value, 4.0);

    let stats = s.stats.unwrap();
    assert_eq!(stats.count, 5);
    assert_eq!(stats.min, 0.0);
    assert_eq!(stats.max, 4.0);
    assert_eq!(stats.mean, 2.0);
    assert_eq!(stats.first, 0.0);
    assert_eq!(stats.last, 4.0);
}

#[test]
fn series_errors_are_typed() {
    let rec = recording();
    let studio = Inspection::new(&rec);

    // Unknown field.
    assert!(matches!(
        studio.series(0, "altitude", &PairDecoder),
        Err(StudioError::FieldNotFound(f)) if f == "altitude"
    ));
    // Unknown channel.
    assert!(matches!(
        studio.series(42, "depth", &PairDecoder),
        Err(StudioError::UnknownChannel(42))
    ));
}

#[test]
fn series_over_an_empty_channel_has_no_stats() {
    // Channel 9 is in the manifest but has no records; it is not indexed, so it
    // reports as an unknown channel for series.
    let rec = recording();
    let studio = Inspection::new(&rec);
    assert!(matches!(
        studio.series(9, "depth", &PairDecoder),
        Err(StudioError::UnknownChannel(9))
    ));
}

#[test]
fn a_single_record_channel_has_no_rate() {
    let mut w = NativeRecordWriter::new(Vec::new(), &manifest()).unwrap();
    w.write_record(
        0,
        0,
        Timestamp::new(ClockDomain::Simulation, 1_000),
        &pair(1.0, 0.0),
    )
    .unwrap();
    let bytes = w.finish().unwrap();
    let rec = NativeRecording::from_bytes(&bytes).unwrap();

    let tl = Inspection::new(&rec).timeline();
    let ch = tl.channels.iter().find(|c| c.id == 0).unwrap();
    assert_eq!(ch.count, 1);
    assert_eq!(ch.span_nanos, Some(0));
    assert_eq!(ch.mean_period_nanos, None);
    assert_eq!(ch.rate_hz, None);
}

#[test]
fn window_with_inverted_range_is_empty_not_a_panic() {
    let rec = recording();
    let studio = Inspection::new(&rec);
    // start > end must yield an empty window, not panic on the slice range.
    assert!(studio.window(0, 60_000_000, 20_000_000).is_empty());
}

#[test]
fn extreme_timestamps_do_not_overflow() {
    // Two records at i128::MIN and i128::MAX on one channel: span and nearest
    // distance must not overflow (the workspace builds with overflow checks on).
    let mut w = NativeRecordWriter::new(Vec::new(), &manifest()).unwrap();
    w.write_record(
        0,
        0,
        Timestamp::new(ClockDomain::Simulation, i128::MIN),
        &pair(0.0, 0.0),
    )
    .unwrap();
    w.write_record(
        0,
        1,
        Timestamp::new(ClockDomain::Simulation, i128::MAX),
        &pair(1.0, 0.0),
    )
    .unwrap();
    let bytes = w.finish().unwrap();
    let rec = NativeRecording::from_bytes(&bytes).unwrap();
    let studio = Inspection::new(&rec);

    // Timeline summary computes a saturated span without panicking.
    let tl = studio.timeline();
    let ch = tl.channels.iter().find(|c| c.id == 0).unwrap();
    assert_eq!(ch.count, 2);
    assert_eq!(ch.span_nanos, Some(i128::MAX)); // saturated

    // nearest at 0 must not overflow and returns one of the two records.
    let n = studio.nearest(0, 0).unwrap();
    assert!(n.timestamp.as_nanos() == i128::MIN || n.timestamp.as_nanos() == i128::MAX);
}

#[test]
fn inspection_is_deterministic() {
    let rec_a = recording();
    let rec_b = recording();
    let a = Inspection::new(&rec_a).timeline();
    let b = Inspection::new(&rec_b).timeline();
    assert_eq!(a, b);
}
