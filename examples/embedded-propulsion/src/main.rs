//! Host simulation of the embedded AUV propulsion node.
//!
//! The same `#![no_std]` `PropulsionNode` that would run on an ESP32-C3 or RP2040
//! is driven here by a deterministic host static loop (no board, no sleeping, no
//! ambient clock). It exercises these local gate scenarios:
//!
//! * a commanded phase where thrust is applied under a valid lease and live link
//!   (slew-limited by the actuator envelope);
//! * **link loss** — the commander goes silent, the watchdog trips, and the node
//!   enters its local safe state on its own;
//! * **lease expiry** — even once the link returns, an expired lease keeps the
//!   node safe.
//!
//! Board integration, durable generation allocation and physical timing/reset
//! validation remain separate requirements; this example runs on the host.
#![forbid(unsafe_code)]

use std::error::Error;

use neuradix_embedded_core::{
    AuthorityLease, Command, CommandGate, CommandMeta, CommandPolicy, EmbeddedComponent,
    Generation, Limits, NodeId, Outcome, PropulsionNode, SafeReason, SessionConfig, SharedTimeline,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};

/// 50 Hz control loop.
const TICK: Duration = Duration::from_millis(20);
/// The authority lease lasts 1.0 s.
const LEASE_NANOS: i128 = 1_000_000_000;
/// The link-loss watchdog: 100 ms without a command trips it.
const WATCHDOG: Duration = Duration::from_millis(100);

fn main() -> Result<(), Box<dyn Error>> {
    println!("Neuradix — embedded AUV propulsion node (host simulation)");
    println!(
        "  lease: {} ms, watchdog: {} ms, tick: {} ms\n",
        LEASE_NANOS / 1_000_000,
        WATCHDOG.as_nanos() / 1_000_000,
        TICK.as_nanos() / 1_000_000,
    );

    // Simulation-only generation. Live startup requires a durable non-reused value.
    let gate = CommandGate::new(
        Limits::new(-1.0, 1.0, 0.2).ok_or("invalid limits")?,
        AuthorityLease::new(
            1,
            2,
            SessionConfig::new(
                Generation::new(1).unwrap(),
                Timestamp::new(ClockDomain::Monotonic, 0),
                Timestamp::new(ClockDomain::Monotonic, LEASE_NANOS),
                CommandPolicy::new(
                    SharedTimeline::new(1, ClockDomain::Monotonic)?,
                    Duration::from_secs(1),
                    Duration::ZERO,
                    WATCHDOG,
                )?,
            )?,
        ),
        0.0, // safe output: zero thrust
    )?;
    let mut node = PropulsionNode::new(NodeId::new("auv/vertical-thruster"), gate);

    let mut now = Timestamp::new(ClockDomain::Monotonic, 0);
    let mut saw_link_loss = false;
    let mut saw_lease_expiry = false;

    println!("  t(ms)  request  applied  outcome                 health");
    for step in 0..70u32 {
        // Commander model: send a rising thrust command for the first 500 ms,
        // then go silent from 500-800 ms (link loss), then resume.
        let ms = now.as_nanos() / 1_000_000;
        let request = if (0..500).contains(&ms) {
            Some((0.05 * step as f32).min(1.0)) // ramp, exercises the slew limit
        } else if (500..800).contains(&ms) {
            None // link loss
        } else {
            Some(0.6) // link restored (but the lease expires at 1000 ms)
        };

        // Source and receiver use the SAME simulated clock. Never stamp a
        // received packet with arrival time as a substitute for source freshness.
        let command = request.map(|value| Command {
            holder: 1,
            capability: 2,
            value,
            meta: CommandMeta {
                generation: Generation::new(1).unwrap(),
                sequence: step as u64,
                source_at: now,
                deadline: now.checked_add(Duration::from_secs(1)).unwrap(),
                timeline: 1,
            },
        });
        let applied = node.tick(now, command);
        let decision = node.last_decision().expect("ticked");

        if let Outcome::SafeState(reason) = decision.outcome {
            if reason == SafeReason::WatchdogExpired {
                saw_link_loss = true;
            }
            if reason == SafeReason::LeaseExpired {
                saw_lease_expiry = true;
            }
        }

        // Print a few representative rows (transitions and endpoints).
        if step % 7 == 0 || matches!(decision.outcome, Outcome::SafeState(_)) {
            println!(
                "  {ms:>5}  {:>7}  {applied:>7.3}  {:<22}  {}",
                request
                    .map(|r| format!("{r:.2}"))
                    .unwrap_or_else(|| "--".to_owned()),
                outcome_label(decision.outcome),
                node.health(),
            );
        }

        now = now.checked_add(TICK)?;
    }

    println!("\nsafety checks");
    println!(
        "  link loss  -> safe state : {}",
        if saw_link_loss { "observed" } else { "MISSING" }
    );
    println!(
        "  lease expiry -> safe state: {}",
        if saw_lease_expiry {
            "observed"
        } else {
            "MISSING"
        }
    );

    if !saw_link_loss {
        return Err("expected a link-loss safe state".into());
    }
    if !saw_lease_expiry {
        return Err("expected a lease-expiry safe state".into());
    }
    println!("\nnode reached its local safe state on both link loss and lease expiry.");
    Ok(())
}

fn outcome_label(outcome: Outcome) -> String {
    match outcome {
        Outcome::Accepted => "accepted".to_owned(),
        Outcome::Modified => "modified (limited)".to_owned(),
        Outcome::SafeState(reason) => format!("SAFE: {reason}"),
    }
}
