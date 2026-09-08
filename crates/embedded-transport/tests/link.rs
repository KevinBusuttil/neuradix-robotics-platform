//! Actual versioned command payload through existing CRC framing into the local gate.
use neuradix_embedded_core::{
    AuthorityLease, Command, CommandGate, CommandMeta, CommandPolicy, EmbeddedComponent,
    Generation, Limits, NodeId, Outcome, PropulsionNode, SafeReason, SessionConfig, SharedTimeline,
};
use neuradix_embedded_transport::{
    COMMAND_BYTES, FrameDecoder, FrameEvent, OVERHEAD, decode_command, encode, encode_command,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
fn t(n: i128) -> Timestamp {
    Timestamp::new(ClockDomain::Monotonic, n)
}
fn command(sequence: u64, source: i128) -> Command {
    Command {
        holder: 1,
        capability: 2,
        value: 0.5,
        meta: CommandMeta {
            generation: Generation::new(7).unwrap(),
            sequence,
            source_at: t(source),
            deadline: t(source + 1_000_000_000),
            timeline: 1,
        },
    }
}
fn node() -> PropulsionNode {
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
        Duration::from_secs(1),
        Duration::ZERO,
        Duration::from_millis(100),
    )
    .unwrap();
    let lease = AuthorityLease::new(
        1,
        2,
        SessionConfig::new(Generation::new(7).unwrap(), t(0), t(10_000_000_000), policy).unwrap(),
    );
    PropulsionNode::new(
        NodeId::new("thruster"),
        CommandGate::new(Limits::new(-1.0, 1.0, 1.0).unwrap(), lease, 0.0).unwrap(),
    )
}
fn frame(transport_seq: u16, command: Command) -> ([u8; OVERHEAD + COMMAND_BYTES], usize) {
    let mut out = [0; OVERHEAD + COMMAND_BYTES];
    let n = encode(transport_seq, &encode_command(command), &mut out).unwrap();
    (out, n)
}
fn receive(decoder: &mut FrameDecoder<128>, bytes: &[u8]) -> Option<Command> {
    let mut command = None;
    for &byte in bytes {
        if let Some(FrameEvent::Frame(_)) = decoder.push(byte) {
            command = decode_command(decoder.payload());
        }
    }
    command
}
#[test]
fn clean_link_preserves_source_metadata_and_applies_commands() {
    let mut node = node();
    let mut decoder = FrameDecoder::<128>::new();
    let original = command(42, 0);
    let (bytes, n) = frame(0, original);
    let received = receive(&mut decoder, &bytes[..n]);
    assert_eq!(received, Some(original));
    assert_eq!(node.tick(t(10_000_000), received), 0.5);
    assert_eq!(
        node.last_decision()
            .unwrap()
            .request
            .unwrap()
            .meta
            .source_at,
        t(0)
    );
    assert_eq!(node.last_decision().unwrap().outcome, Outcome::Accepted);
}
#[test]
fn sustained_corruption_cannot_feed_the_actuator_watchdog() {
    let mut node = node();
    let mut decoder = FrameDecoder::<128>::new();
    let mut entered = None;
    for i in 0..20u16 {
        let now = i as i128 * 20_000_000;
        let (mut bytes, n) = frame(i, command(i as u64, now));
        if now >= 60_000_000 {
            bytes[7] ^= 1;
        }
        node.tick(t(now), receive(&mut decoder, &bytes[..n]));
        if node.in_safe_state() && entered.is_none() {
            entered = Some(now);
            assert_eq!(
                node.last_decision().unwrap().outcome,
                Outcome::SafeState(SafeReason::WatchdogExpired)
            );
        }
    }
    assert_eq!(entered, Some(160_000_000));
    assert!(node.in_safe_state());
}
#[test]
fn fresh_outer_frame_numbers_do_not_make_replayed_commands_fresh() {
    let mut node = node();
    let mut decoder = FrameDecoder::<128>::new();
    let original = command(0, 0);
    for frame_seq in 0..8 {
        let (bytes, n) = frame(frame_seq, original);
        node.tick(
            t(frame_seq as i128 * 20_000_000),
            receive(&mut decoder, &bytes[..n]),
        );
        if frame_seq > 0 {
            assert_eq!(
                node.last_decision().unwrap().outcome,
                Outcome::SafeState(SafeReason::Duplicate)
            );
        }
    }
    node.tick(t(160_000_000), None);
    assert_eq!(
        node.last_decision().unwrap().outcome,
        Outcome::SafeState(SafeReason::WatchdogExpired)
    );
}
#[test]
fn unsupported_payloads_never_become_arrival_stamped_commands() {
    assert!(decode_command(&0.5f32.to_le_bytes()).is_none()); // incompatible legacy format
    let original = encode_command(command(0, 0));
    for size in [0, COMMAND_BYTES - 1] {
        assert!(decode_command(&original[..size]).is_none());
    }
    let mut bad = original;
    bad[0] = 2;
    assert!(decode_command(&bad).is_none());
    let mut bad = original;
    bad[49] = 255;
    assert!(decode_command(&bad).is_none());
    let mut bad = original;
    bad[17..33].fill(0);
    assert!(decode_command(&bad).is_none());
    let mut bad_value = command(0, 0);
    bad_value.value = f32::NAN;
    let received = decode_command(&encode_command(bad_value)).unwrap();
    assert!(received.value.is_nan()); // numeric rejection remains auditable at gate
}
#[test]
fn delayed_previous_generation_and_stale_payloads_are_rejected_after_decode() {
    let mut decoder = FrameDecoder::<128>::new();
    let mut node = node();
    let mut old = command(0, 0);
    old.meta.generation = Generation::new(6).unwrap();
    let (bytes, n) = frame(0, old);
    node.tick(t(0), receive(&mut decoder, &bytes[..n]));
    assert_eq!(
        node.last_decision().unwrap().outcome,
        Outcome::SafeState(SafeReason::GenerationMismatch)
    );
    let mut stale = command(1, 0);
    stale.meta.deadline = t(3_000_000_000);
    let (bytes, n) = frame(1, stale);
    node.tick(t(2_000_000_000), receive(&mut decoder, &bytes[..n]));
    assert_eq!(
        node.last_decision().unwrap().outcome,
        Outcome::SafeState(SafeReason::StaleCommand)
    );
}
