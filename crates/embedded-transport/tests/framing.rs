//! Framing, CRC, resync and sequence-tracking tests.

use neuradix_embedded_transport::{
    Frame, FrameDecoder, FrameEvent, OVERHEAD, SeqStatus, SequenceTracker, crc32, encode,
};

/// Feed a whole byte buffer to a decoder, collecting every event.
fn decode_all<const N: usize>(
    dec: &mut FrameDecoder<N>,
    bytes: &[u8],
) -> Vec<(FrameEvent, Vec<u8>)> {
    let mut out = Vec::new();
    for &b in bytes {
        if let Some(ev) = dec.push(b) {
            out.push((ev, dec.payload().to_vec()));
        }
    }
    out
}

#[test]
fn crc32_matches_the_standard_check_value() {
    // The canonical CRC-32 check value.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn a_frame_round_trips() {
    let payload = [10u8, 20, 30, 40];
    let mut wire = [0u8; OVERHEAD + 4];
    let n = encode(42, &payload, &mut wire).unwrap();
    assert_eq!(n, OVERHEAD + 4);

    let mut dec = FrameDecoder::<64>::new();
    let events = decode_all(&mut dec, &wire[..n]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, FrameEvent::Frame(Frame { seq: 42, len: 4 }));
    assert_eq!(events[0].1, payload);
}

#[test]
fn an_empty_payload_round_trips() {
    let mut wire = [0u8; OVERHEAD];
    let n = encode(1, &[], &mut wire).unwrap();
    let mut dec = FrameDecoder::<8>::new();
    let events = decode_all(&mut dec, &wire[..n]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, FrameEvent::Frame(Frame { seq: 1, len: 0 }));
    assert!(events[0].1.is_empty());
}

#[test]
fn a_single_bit_flip_is_detected() {
    let payload = [1u8, 2, 3, 4, 5];
    let mut wire = [0u8; OVERHEAD + 5];
    let n = encode(3, &payload, &mut wire).unwrap();
    // Flip a bit in the payload region.
    wire[7] ^= 0x01;

    let mut dec = FrameDecoder::<64>::new();
    let events = decode_all(&mut dec, &wire[..n]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, FrameEvent::Corrupt, "CRC must reject the flip");
}

#[test]
fn the_decoder_resyncs_after_leading_garbage() {
    let payload = [7u8, 8, 9];
    let mut wire = [0u8; OVERHEAD + 3];
    let n = encode(5, &payload, &mut wire).unwrap();

    let mut dec = FrameDecoder::<32>::new();
    // Prepend noise (including a stray 0xAA to test the sync state machine).
    let mut stream = vec![0x00, 0xFF, 0xAA, 0x13, 0xAA];
    stream.extend_from_slice(&wire[..n]);

    let events = decode_all(&mut dec, &stream);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, FrameEvent::Frame(Frame { seq: 5, len: 3 }));
    assert_eq!(events[0].1, payload);
}

#[test]
fn two_back_to_back_frames_both_decode() {
    let mut wire = [0u8; 2 * (OVERHEAD + 2)];
    let a = encode(1, &[0xAA, 0x55], &mut wire).unwrap(); // payload equal to sync bytes
    let b = encode(2, &[0x00, 0x01], &mut wire[a..]).unwrap();

    let mut dec = FrameDecoder::<16>::new();
    let events = decode_all(&mut dec, &wire[..a + b]);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].0, FrameEvent::Frame(Frame { seq: 1, len: 2 }));
    assert_eq!(events[0].1, [0xAA, 0x55]);
    assert_eq!(events[1].0, FrameEvent::Frame(Frame { seq: 2, len: 2 }));
}

#[test]
fn an_oversized_frame_is_dropped_not_overflowed() {
    // Encode a 10-byte payload, decode with a buffer that only holds 4.
    let payload = [0u8; 10];
    let mut wire = [0u8; OVERHEAD + 10];
    let n = encode(1, &payload, &mut wire).unwrap();
    let mut dec = FrameDecoder::<4>::new();
    let events = decode_all(&mut dec, &wire[..n]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, FrameEvent::Corrupt);
}

#[test]
fn encode_rejects_too_small_a_buffer() {
    let mut small = [0u8; 4];
    assert!(encode(0, &[1, 2, 3], &mut small).is_err());
}

#[test]
fn sequence_tracker_classifies_order_gaps_and_duplicates() {
    let mut t = SequenceTracker::new();
    assert_eq!(t.observe(10), SeqStatus::First);
    assert_eq!(t.observe(11), SeqStatus::InOrder);
    assert_eq!(t.observe(11), SeqStatus::Duplicate);
    assert_eq!(t.observe(14), SeqStatus::Gap(2)); // 12 and 13 missed
    assert_eq!(t.observe(13), SeqStatus::Reordered); // late frame
    assert_eq!(t.last(), Some(14));
}

#[test]
fn sequence_tracker_handles_the_u16_wrap() {
    let mut t = SequenceTracker::new();
    assert_eq!(t.observe(u16::MAX), SeqStatus::First);
    assert_eq!(t.observe(0), SeqStatus::InOrder); // wraps to 0
    assert_eq!(t.observe(2), SeqStatus::Gap(1)); // 1 missed
}
