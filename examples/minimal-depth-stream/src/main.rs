//! Minimal depth-stream example.
//!
//! The first small proof of the Neuradix architecture end to end:
//!
//! ```text
//! VehicleDepth producer -> bounded in-process stream -> VehicleDepth consumer
//!                       -> deterministic recording -> replay (verified)
//! ```
//!
//! It demonstrates, with no network, no threads sleeping and no ambient clock:
//!
//! * a generated, contract-derived Rust type (`generated::vehicle_depth`);
//! * an explicit timestamp carrying its clock domain on every sample;
//! * queue capacity and overflow policy taken from the authored contract;
//! * components moved through valid lifecycle states;
//! * recording the run and replaying it with verified, byte-stable fidelity;
//! * fully deterministic execution and termination.
#![forbid(unsafe_code)]

use std::error::Error;
use std::path::Path;

use neuradix_contracts::{ClockDomainRef, schema_identity, validate};
use neuradix_record::{
    Channel, NativeRecordWriter, NativeRecording, RecordCodec, RecordError, RecordingManifest,
    SoftwareId, replay_digest,
};
use neuradix_runtime::{
    Component, ComponentError, ComponentId, HealthState, Lifecycle, LifecycleState, Processor,
    TickContext, run_lockstep,
};
use neuradix_safety::{
    AuthorityLease, Capability, CommandLineage, CommandRequest, Constraint, Identity,
    LINEAGE_CHANNEL, LeaseTable, LineageOrigin, Outcome, SafetyGate,
};
use neuradix_time::{Clock, ClockDomain, Duration, ManualClock, Timestamp};
use neuradix_transport_api::{
    PublishOutcome, StreamConfig, StreamPublisher, StreamSubscriber, in_process,
};

mod generated;
use generated::vehicle_depth::VehicleDepth;

/// The authored contract, embedded so the example is self-contained.
const CONTRACT_YAML: &str =
    include_str!("../../../contracts/standard/navigation/vehicle-depth.yaml");

/// The single channel id used by this mission recording.
const CHANNEL_ID: u16 = 0;

/// A depth measurement plus the domain-tagged time at which it was measured.
#[derive(Clone, Copy, Debug)]
struct DepthSample {
    measurement_time: Timestamp,
    value: VehicleDepth,
}

/// A deterministic fixed-layout codec for `VehicleDepth` (two little-endian f64).
struct DepthCodec;

impl RecordCodec for DepthCodec {
    type Message = VehicleDepth;

    fn encode(&self, m: &VehicleDepth) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        out.extend_from_slice(&m.depth.to_le_bytes());
        out.extend_from_slice(&m.uncertainty.to_le_bytes());
        out
    }

    fn decode(&self, bytes: &[u8]) -> Result<VehicleDepth, RecordError> {
        if bytes.len() != 16 {
            return Err(RecordError::Decode(format!(
                "expected 16 bytes, got {}",
                bytes.len()
            )));
        }
        let depth = f64::from_le_bytes(bytes[0..8].try_into().expect("8 bytes"));
        let uncertainty = f64::from_le_bytes(bytes[8..16].try_into().expect("8 bytes"));
        Ok(VehicleDepth { depth, uncertainty })
    }
}

/// A thrust command produced by the controller, tagged with its decision time.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ThrustCommand {
    at: Timestamp,
    thrust: f64,
}

/// A minimal proportional depth controller used to demonstrate that control
/// decisions replay identically from a recording.
struct DepthController {
    kp: f64,
    setpoint: f64,
    max_thrust: f64,
}

impl Processor for DepthController {
    type Input = VehicleDepth;
    type Output = ThrustCommand;

    fn process(
        &mut self,
        ctx: &TickContext,
        depth: VehicleDepth,
    ) -> Result<Vec<ThrustCommand>, ComponentError> {
        let error = self.setpoint - depth.depth;
        let thrust = (self.kp * error).clamp(-self.max_thrust, self.max_thrust);
        Ok(vec![ThrustCommand {
            at: ctx.now,
            thrust,
        }])
    }
}

/// A component that produces depth samples.
struct DepthProducer {
    id: ComponentId,
}

impl Component for DepthProducer {
    fn id(&self) -> &ComponentId {
        &self.id
    }
    fn health(&self) -> HealthState {
        HealthState::Healthy
    }
}

/// A component that consumes depth samples.
struct DepthConsumer {
    id: ComponentId,
    last_seen: Option<DepthSample>,
    received: u64,
}

impl DepthConsumer {
    fn drain(&mut self, rx: &impl StreamSubscriber<DepthSample>) {
        while let Some(sample) = rx.poll() {
            self.received += 1;
            self.last_seen = Some(sample);
        }
    }
}

impl Component for DepthConsumer {
    fn id(&self) -> &ComponentId {
        &self.id
    }
    fn health(&self) -> HealthState {
        HealthState::Healthy
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // 1. Contract-driven configuration.
    let contract = validate::from_yaml_str(CONTRACT_YAML, Path::new("vehicle-depth.yaml"))?;
    let config = StreamConfig::from_delivery(&contract.spec.delivery);
    let domain = map_domain(contract.spec.semantics.clock_domain);
    let schema = schema_identity(&contract);

    println!("Neuradix — minimal depth-stream example");
    println!(
        "  contract : {} v{}",
        contract.identifier(),
        contract.metadata.version
    );
    println!("  schema   : {schema}");
    println!(
        "  stream   : capacity={}, overflow={}, clockDomain={}",
        config.capacity,
        contract.spec.delivery.overflow.as_str(),
        domain
    );

    // 2. Deterministic injected clock (no ambient time, no sleeping).
    let clock = ManualClock::new(Timestamp::new(domain, 0));

    // 3. Bounded in-process stream and the two components.
    let (tx, rx) = in_process::<DepthSample>(config);
    let mut producer = DepthProducer {
        id: ComponentId::new("depth-producer")?,
    };
    let mut consumer = DepthConsumer {
        id: ComponentId::new("depth-consumer")?,
        last_seen: None,
        received: 0,
    };
    let mut producer_lc = Lifecycle::new();
    let mut consumer_lc = Lifecycle::new();

    // 4. Bring both components up through valid lifecycle states.
    println!("\nlifecycle: bringing components up");
    bring_up(&mut producer, &mut producer_lc, &clock)?;
    bring_up(&mut consumer, &mut consumer_lc, &clock)?;

    // Collect every published sample so the mission can be recorded and replayed.
    let mut published: Vec<DepthSample> = Vec::new();

    // 5. Phase A — a flowing pipeline: publish then consume each step.
    println!("\nphase A — flowing pipeline (publish then consume)");
    for step in 0..5 {
        clock.advance(Duration::from_millis(50))?;
        let sample = make_sample(step, &clock);
        published.push(sample);
        let outcome = tx.publish(sample)?;
        consumer.drain(&rx);
        println!(
            "  t={} depth={:.2}m outcome={:?}",
            sample.measurement_time, sample.value.depth, outcome
        );
    }

    // 6. Phase B — a burst larger than the queue, to exercise overflow.
    println!("\nphase B — burst of 10 without draining (overflow policy in effect)");
    for step in 5..15 {
        clock.advance(Duration::from_millis(50))?;
        let sample = make_sample(step, &clock);
        published.push(sample);
        let outcome = tx.publish(sample)?;
        if outcome != PublishOutcome::Enqueued {
            println!("  step {step}: {outcome:?}");
        }
    }
    consumer.drain(&rx);

    // 7. Shut both components down through valid lifecycle states.
    println!("\nlifecycle: shutting components down");
    shut_down(&mut producer, &mut producer_lc, &clock)?;
    shut_down(&mut consumer, &mut consumer_lc, &clock)?;

    // 8. Report stream statistics and the final delivered sample.
    let stats = rx.stats();
    println!("\nstream statistics");
    println!("  capacity  : {}", stats.capacity);
    println!("  published : {}", stats.published);
    println!("  delivered : {}", stats.delivered);
    println!("  dropped   : {}", stats.dropped);
    println!("  rejected  : {}", stats.rejected);
    println!("  final len : {}", stats.len);
    println!("  consumer received : {}", consumer.received);

    // 9. Record the mission and prove deterministic replay.
    println!("\nrecord & replay");
    let bytes = record_mission(&schema, domain, &published)?;
    let recording = NativeRecording::from_bytes(&bytes)?;
    let digest = replay_digest(&recording);

    // Replay: decode every recorded payload and check it matches what was sent.
    let codec = DepthCodec;
    let replayed: Vec<VehicleDepth> = recording
        .records_for(CHANNEL_ID)
        .map(|r| codec.decode(&r.payload))
        .collect::<Result<_, _>>()?;
    let originals: Vec<VehicleDepth> = published.iter().map(|s| s.value).collect();
    let verified = replayed == originals;

    println!("  records  : {}", recording.records().len());
    println!("  digest   : {digest}");
    println!(
        "  fidelity : {}",
        if verified {
            "verified (replay == recorded)"
        } else {
            "MISMATCH"
        }
    );

    let path = std::env::temp_dir().join("neuradix-depth-mission.nrec");
    std::fs::write(&path, &bytes)?;
    println!("  written  : {}", path.display());
    println!("  inspect  : neuradix record inspect {}", path.display());
    println!(
        "  replay   : neuradix replay run {} --expect-digest {digest}",
        path.display()
    );

    if !verified {
        return Err("replay fidelity check failed".into());
    }

    // 10. Deterministic control + lockstep replay equivalence.
    //
    // Run a depth controller live over the samples that were sent, then run the
    // same controller over the samples decoded from the recording, each under a
    // fresh clock. If the control *decisions* match, the system (not just the
    // data) replays identically.
    println!("\ndeterministic control & lockstep replay");

    let live_clock = ManualClock::new(Timestamp::new(domain, 0));
    let live_inputs = published.iter().map(|s| (s.measurement_time, s.value));
    let thrust_live = run_lockstep(&live_clock, &mut new_controller(), live_inputs)?;

    let replay_clock = ManualClock::new(Timestamp::new(domain, 0));
    let replay_inputs: Vec<(Timestamp, VehicleDepth)> = recording
        .records_for(CHANNEL_ID)
        .map(|r| codec.decode(&r.payload).map(|value| (r.timestamp, value)))
        .collect::<Result<_, _>>()?;
    let thrust_replay = run_lockstep(&replay_clock, &mut new_controller(), replay_inputs)?;

    let control_equivalent = thrust_live == thrust_replay;
    println!("  commands : {}", thrust_live.len());
    if let Some(last) = thrust_live.last() {
        println!("  last cmd : thrust={:.3} at {}", last.thrust, last.at);
    }
    println!(
        "  lockstep : {}",
        if control_equivalent {
            "verified (live control == replayed control)"
        } else {
            "MISMATCH"
        }
    );
    if !control_equivalent {
        return Err("lockstep control replay mismatch".into());
    }

    // 11. Route the control commands through the safety authority + constraint
    // path. The lease is valid only for the first half of the mission, so later
    // commands are rejected and forced to the fail-safe output; a tight range
    // clamps the largest thrust demands. Every decision is auditable.
    println!("\nsafety authority & constraints");
    let holder = Identity::new("depth-controller");
    let capability = Capability::new("propulsion/vertical-thrust");
    let mut leases = LeaseTable::new();
    leases.grant(AuthorityLease {
        holder: holder.clone(),
        capability: capability.clone(),
        priority: 10,
        issued: Timestamp::new(domain, 0),
        // Authority lapses halfway through the mission (at 400ms).
        expires: Timestamp::new(domain, 400_000_000),
        envelope: None,
    });
    let constraints = vec![
        Constraint::range("thrust-range", -0.8, 0.8)?,
        Constraint::slew_rate("thrust-slew", 50.0)?,
    ];
    let mut gate = SafetyGate::new(leases, constraints, 0.0);

    let safety_clock = ManualClock::new(Timestamp::new(domain, 0));
    let requests = thrust_live.iter().map(|c| {
        (
            c.at,
            CommandRequest::new(holder.clone(), capability.clone(), c.thrust, c.at),
        )
    });
    let decisions = run_lockstep(&safety_clock, &mut gate, requests)?;

    let (mut accepted, mut modified, mut rejected) = (0u32, 0u32, 0u32);
    for d in &decisions {
        match d.outcome {
            Outcome::Accepted => accepted += 1,
            Outcome::Modified => modified += 1,
            Outcome::Rejected(_) => rejected += 1,
        }
    }
    println!(
        "  decisions: {} (accepted {accepted}, modified {modified}, rejected {rejected})",
        decisions.len()
    );
    for d in decisions
        .iter()
        .filter(|d| d.outcome != Outcome::Accepted)
        .take(4)
    {
        let rules = if d.acted_rules.is_empty() {
            d.outcome.label().to_owned()
        } else {
            d.acted_rules.join(",")
        };
        println!(
            "  t={} requested={:.2} -> applied={:.2} [{}]",
            d.at, d.request.value, d.applied, rules
        );
    }
    println!(
        "  note     : rejected commands apply the fail-safe output (0.0) after authority lapses"
    );

    // 12. Record the command lineage so any actuator command can be explained.
    // Each lineage entry links the originating depth sample to the controller
    // request, the authority/constraint outcome and the applied value.
    println!("\ncommand lineage");
    let lineage_manifest = RecordingManifest::builder("neuradix-example-minimal-depth-stream")
        .channel(Channel {
            id: 0,
            name: LINEAGE_CHANNEL.to_owned(),
            schema_id: "application/vnd.neuradix.command-lineage+json".to_owned(),
            clock_domain: domain.as_str().to_owned(),
        })
        .software(SoftwareId::new(
            "neuradix-example-minimal-depth-stream",
            env!("CARGO_PKG_VERSION"),
        ))
        .note("depth mission command lineage")
        .build();
    let mut lineage_writer = NativeRecordWriter::new(Vec::new(), &lineage_manifest)?;
    for (trace, (sample, decision)) in published.iter().zip(decisions.iter()).enumerate() {
        let origin =
            LineageOrigin::new("navigation/vehicle-depth", "depth", "m", sample.value.depth);
        let lineage = CommandLineage::from_decision(trace as u64, origin, decision);
        lineage_writer.write_record(0, trace as u64, decision.at, &lineage.to_json_bytes())?;
    }
    let lineage_bytes = lineage_writer.finish()?;
    let lineage_path = std::env::temp_dir().join("neuradix-depth-lineage.nrec");
    std::fs::write(&lineage_path, &lineage_bytes)?;
    println!("  entries  : {}", decisions.len());
    println!("  written  : {}", lineage_path.display());
    println!(
        "  explain  : neuradix explain command {} --at 50000000",
        lineage_path.display()
    );
    println!(
        "  explain  : neuradix explain command {} --at 450000000  (a rejected command)",
        lineage_path.display()
    );

    println!("\ndone.");
    Ok(())
}

/// A fresh depth controller with the mission's control parameters.
fn new_controller() -> DepthController {
    DepthController {
        kp: 0.5,
        setpoint: 12.0,
        max_thrust: 5.0,
    }
}

/// Build a deterministic sample for `step`, stamped with the current sim time.
fn make_sample(step: u32, clock: &ManualClock) -> DepthSample {
    DepthSample {
        measurement_time: clock.now(),
        value: VehicleDepth {
            depth: 10.0 + f64::from(step) * 0.5,
            uncertainty: 0.1,
        },
    }
}

/// Encode the whole mission into a native recording buffer.
fn record_mission(
    schema: &neuradix_contracts::SchemaId,
    domain: ClockDomain,
    samples: &[DepthSample],
) -> Result<Vec<u8>, Box<dyn Error>> {
    let manifest = RecordingManifest::builder("neuradix-example-minimal-depth-stream")
        .channel(Channel::new(
            CHANNEL_ID,
            "navigation/vehicle-depth",
            schema,
            domain,
        ))
        .software(SoftwareId::new(
            "neuradix-example-minimal-depth-stream",
            env!("CARGO_PKG_VERSION"),
        ))
        .note("minimal depth mission")
        .build();

    let codec = DepthCodec;
    let mut writer = NativeRecordWriter::new(Vec::new(), &manifest)?;
    for (seq, sample) in samples.iter().enumerate() {
        writer.write_record(
            CHANNEL_ID,
            seq as u64,
            sample.measurement_time,
            &codec.encode(&sample.value),
        )?;
    }
    Ok(writer.finish()?)
}

/// Drive a component `Declared -> Configured -> Inactive -> Active`.
fn bring_up(
    component: &mut impl Component,
    lifecycle: &mut Lifecycle,
    clock: &ManualClock,
) -> Result<(), Box<dyn Error>> {
    step(
        lifecycle,
        LifecycleState::Configured,
        "configuration validated",
        clock,
    )?;
    component.on_configure()?;
    step(
        lifecycle,
        LifecycleState::Inactive,
        "contracts resolved",
        clock,
    )?;
    step(lifecycle, LifecycleState::Active, "activated", clock)?;
    component.on_activate()?;
    Ok(())
}

/// Drive a component `Active -> Stopping -> Stopped`.
fn shut_down(
    component: &mut impl Component,
    lifecycle: &mut Lifecycle,
    clock: &ManualClock,
) -> Result<(), Box<dyn Error>> {
    step(
        lifecycle,
        LifecycleState::Stopping,
        "shutdown requested",
        clock,
    )?;
    component.on_stop()?;
    step(lifecycle, LifecycleState::Stopped, "stopped", clock)?;
    Ok(())
}

/// Apply and print one lifecycle transition.
fn step(
    lifecycle: &mut Lifecycle,
    to: LifecycleState,
    reason: &str,
    clock: &ManualClock,
) -> Result<(), ComponentError> {
    let record = lifecycle
        .transition(to, reason, "example-driver", clock.now())
        .map_err(|e| ComponentError::Failed(e.to_string()))?;
    println!("  {} -> {} ({})", record.from, record.to, record.reason);
    Ok(())
}

/// Map the contract clock-domain vocabulary onto `neuradix-time`.
fn map_domain(domain: ClockDomainRef) -> ClockDomain {
    match domain {
        ClockDomainRef::Monotonic => ClockDomain::Monotonic,
        ClockDomainRef::Utc => ClockDomain::Utc,
        ClockDomainRef::Sensor => ClockDomain::Sensor,
        ClockDomainRef::Simulation => ClockDomain::Simulation,
        ClockDomainRef::Replay => ClockDomain::Replay,
    }
}
