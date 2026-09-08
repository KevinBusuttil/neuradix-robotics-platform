//! Integration: a framed command link feeding the embedded propulsion node.
//!
//! Demonstrates the WP4 ↔ WP2 tie-in — a corrupted frame is rejected by the CRC,
//! so no fresh command reaches the node, and once the corruption outlasts the
//! watchdog the node falls back to its **local safe state**. A noisy link can
//! never drive an unsafe command through.

use neuradix_embedded_core::{
    AuthorityLease, CommandGate, EmbeddedComponent, Limits, NodeId, Outcome, PropulsionNode,
    SafeReason, Watchdog,
};
use neuradix_embedded_transport::{FrameDecoder, FrameEvent, OVERHEAD, encode};
use neuradix_time::{ClockDomain, Duration, Timestamp};

fn t(ns: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, ns)
}

/// Encode a thrust command (`f32` little-endian) into a frame.
fn frame_command(seq: u16, thrust: f32) -> ([u8; OVERHEAD + 4], usize) {
    let mut buf = [0u8; OVERHEAD + 4];
    let n = encode(seq, &thrust.to_le_bytes(), &mut buf).unwrap();
    (buf, n)
}

/// Push a frame's bytes through the decoder and, if a verified frame completes,
/// decode the `f32` thrust command it carries.
fn receive(decoder: &mut FrameDecoder<32>, bytes: &[u8]) -> Option<f32> {
    let mut command = None;
    for &b in bytes {
        if let Some(FrameEvent::Frame(_)) = decoder.push(b) {
            let p = decoder.payload();
            if p.len() == 4 {
                command = Some(f32::from_le_bytes([p[0], p[1], p[2], p[3]]));
            }
        }
    }
    command
}

#[test]
fn a_clean_link_applies_commands() {
    let gate = CommandGate::new(
        Limits::new(-1.0, 1.0, 1.0).unwrap(),
        AuthorityLease::until(t(10_000_000_000)),
        Watchdog::new(Duration::from_millis(100)),
        0.0,
    );
    let mut node = PropulsionNode::new(NodeId::new("thruster"), gate);
    let mut decoder = FrameDecoder::<32>::new();

    let (buf, n) = frame_command(0, 0.5);
    let command = receive(&mut decoder, &buf[..n]);
    assert_eq!(node.tick(t(0), command), 0.5);
    assert_eq!(node.last_decision().unwrap().outcome, Outcome::Accepted);
}

#[test]
fn sustained_corruption_drives_the_node_to_safe_state() {
    let gate = CommandGate::new(
        Limits::new(-1.0, 1.0, 1.0).unwrap(),
        AuthorityLease::until(t(10_000_000_000)),
        Watchdog::new(Duration::from_millis(100)), // 100 ms link timeout
        0.0,
    );
    let mut node = PropulsionNode::new(NodeId::new("thruster"), gate);
    let mut decoder = FrameDecoder::<32>::new();

    let step = 20_000_000i128; // 20 ms ticks (50 Hz)
    let mut entered_safe_at = None;

    for i in 0..20u16 {
        let now = t(i as i128 * step);
        let (mut buf, n) = frame_command(i, 0.6);

        // From 60 ms onward every frame is corrupted (a bit flipped in the
        // payload), so the CRC rejects it and no command is delivered.
        if now.as_nanos() >= 60_000_000 {
            buf[7] ^= 0x01;
        }

        let command = receive(&mut decoder, &buf[..n]);
        node.tick(now, command);

        if node.in_safe_state() && entered_safe_at.is_none() {
            entered_safe_at = Some(now.as_nanos());
            assert_eq!(
                node.last_decision().unwrap().outcome,
                Outcome::SafeState(SafeReason::LinkLost),
            );
        }
    }

    // Corruption started at 60 ms; the 100 ms watchdog trips ~160 ms in.
    let at = entered_safe_at.expect("node must reach safe state under corruption");
    assert!(
        at >= 160_000_000,
        "safe state should follow the watchdog timeout, entered at {at} ns"
    );
    assert!(node.in_safe_state());
}
