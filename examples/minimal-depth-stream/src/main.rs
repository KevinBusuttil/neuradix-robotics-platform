//! Minimal depth-stream example.
//!
//! The first small proof of the Neuradix architecture end to end:
//!
//! ```text
//! VehicleDepth producer -> bounded in-process stream -> VehicleDepth consumer
//! ```
//!
//! It demonstrates, with no network, no threads sleeping and no ambient clock:
//!
//! * a generated, contract-derived Rust type (`generated::vehicle_depth`);
//! * an explicit timestamp carrying its clock domain on every sample;
//! * queue capacity and overflow policy taken from the authored contract;
//! * components moved through valid lifecycle states;
//! * bounded-stream statistics printed at the end;
//! * fully deterministic execution and termination.
#![forbid(unsafe_code)]

use std::error::Error;
use std::path::Path;

use neuradix_contracts::{ClockDomainRef, schema_identity, validate};
use neuradix_runtime::{
    Component, ComponentError, ComponentId, HealthState, Lifecycle, LifecycleState,
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

/// A depth measurement plus the domain-tagged time at which it was measured.
#[derive(Clone, Copy, Debug)]
struct DepthSample {
    measurement_time: Timestamp,
    value: VehicleDepth,
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

    println!("Neuradix — minimal depth-stream example");
    println!(
        "  contract : {} v{}",
        contract.identifier(),
        contract.metadata.version
    );
    println!("  schema   : {}", schema_identity(&contract));
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

    // 5. Phase A — a flowing pipeline: publish then consume each step.
    println!("\nphase A — flowing pipeline (publish then consume)");
    for step in 0..5 {
        clock.advance(Duration::from_millis(50))?;
        let sample = make_sample(step, &clock);
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
        let outcome = tx.publish(make_sample(step, &clock))?;
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
    match consumer.last_seen {
        Some(sample) => println!(
            "  last sample: depth={:.2}m uncertainty={:.2}m at {} (domain {})",
            sample.value.depth,
            sample.value.uncertainty,
            sample.measurement_time,
            sample.measurement_time.domain()
        ),
        None => println!("  last sample: none"),
    }

    println!("\ndone.");
    Ok(())
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
