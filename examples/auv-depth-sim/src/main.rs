//! Closed-loop AUV depth mission.
//!
//! This is the vertical slice run against a *simulated vehicle*: the loop is
//!
//! ```text
//! DepthSensor -> proportional control -> SafetyGate -> DepthPlant -> (repeat)
//! ```
//!
//! Every actuator command is produced by a control law but only ever reaches the
//! plant *after* passing through the safety authority + constraint gate, exactly
//! as it would on a real vehicle. The whole mission is deterministic — no
//! network, no threads, no ambient clock — so running it twice yields a
//! byte-identical trajectory, confirmed here with a content digest.
#![forbid(unsafe_code)]

use std::error::Error;

use neuradix_safety::{
    AuthorityLease, Capability, CommandRequest, Constraint, Identity, LeaseTable, Outcome,
    SafetyGate,
};
use neuradix_sim::{
    Controller, DepthPlant, DepthSensor, PlantParams, PlantState, Simulation, StepContext,
    Trajectory,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
use sha2::{Digest, Sha256};

/// The mission's control frequency: 50 Hz.
const STEP: Duration = Duration::from_millis(20);
/// Target depth for the descent (m).
const SETPOINT: f64 = 6.0;
/// Number of control steps (30 s at 50 Hz).
const STEPS: u64 = 1_500;

/// A proportional depth controller whose every command is gated by the safety
/// authority before it is applied. This is the seam that lets the *real* control
/// and safety path drive the simulated plant.
struct SafetyGatedController {
    gain: f64,
    setpoint: f64,
    gate: SafetyGate,
    holder: Identity,
    capability: Capability,
    accepted: u32,
    modified: u32,
    rejected: u32,
}

impl SafetyGatedController {
    fn new() -> Self {
        let holder = Identity::new("depth-controller");
        let capability = Capability::new("propulsion/vertical-thrust");

        let mut leases = LeaseTable::new();
        leases.grant(AuthorityLease {
            holder: holder.clone(),
            capability: capability.clone(),
            priority: 10,
            issued: Timestamp::new(ClockDomain::Simulation, 0),
            // Authority for the whole mission.
            expires: Timestamp::new(ClockDomain::Simulation, i128::MAX),
            envelope: None,
        });

        // The raw control law may demand far more than the actuator envelope; the
        // range constraint clamps it to +/-0.8, and the slew constraint bounds
        // how fast the command may change.
        let constraints = vec![
            Constraint::range("thrust-range", -0.8, 0.8).expect("valid range"),
            Constraint::slew_rate("thrust-slew", 50.0).expect("valid slew"),
        ];

        Self {
            gain: 0.6,
            setpoint: SETPOINT,
            gate: SafetyGate::new(leases, constraints, 0.0),
            holder,
            capability,
            accepted: 0,
            modified: 0,
            rejected: 0,
        }
    }
}

impl Controller for SafetyGatedController {
    fn command(&mut self, ctx: &StepContext, measured_depth: f64) -> f64 {
        // Proportional law: descend when shallower than the setpoint.
        let raw = self.gain * (self.setpoint - measured_depth);

        let request =
            CommandRequest::new(self.holder.clone(), self.capability.clone(), raw, ctx.now);
        let decision = self.gate.evaluate(request);
        match decision.outcome {
            Outcome::Accepted => self.accepted += 1,
            Outcome::Modified => self.modified += 1,
            Outcome::Rejected(_) => self.rejected += 1,
        }
        decision.applied
    }
}

/// A tally of how the safety gate handled the mission's commands.
struct SafetyTally {
    accepted: u32,
    modified: u32,
    rejected: u32,
}

/// Run one full mission, returning the trajectory and the safety-decision tally.
fn run_mission() -> Result<(Trajectory, SafetyTally), Box<dyn Error>> {
    let plant = DepthPlant::new(PlantParams::default(), PlantState::at_surface())?;
    let mut sim = Simulation::new(
        Timestamp::new(ClockDomain::Simulation, 0),
        STEP,
        plant,
        DepthSensor::ideal(),
        SafetyGatedController::new(),
    )?;
    let trajectory = sim.run(STEPS)?;
    let c = sim.controller();
    Ok((
        trajectory,
        SafetyTally {
            accepted: c.accepted,
            modified: c.modified,
            rejected: c.rejected,
        },
    ))
}

/// A content digest over the trajectory, so determinism is verifiable.
fn digest(trajectory: &Trajectory) -> String {
    let mut hasher = Sha256::new();
    for s in &trajectory.samples {
        hasher.update(s.step.to_le_bytes());
        hasher.update(s.time.as_nanos().to_le_bytes());
        hasher.update(s.true_depth.to_le_bytes());
        hasher.update(s.measured_depth.to_le_bytes());
        hasher.update(s.velocity.to_le_bytes());
        hasher.update(s.thrust.to_le_bytes());
    }
    hasher.update(trajectory.final_state.depth.to_le_bytes());
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256:{hex}")
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("closed-loop AUV depth mission (control -> safety -> simulated plant)");
    println!(
        "  setpoint : {SETPOINT:.1} m, step: {} ms, steps: {STEPS}",
        STEP.as_nanos() / 1_000_000
    );

    let (trajectory, tally) = run_mission()?;

    // Progress snapshots at a few points in the descent.
    println!("\ntrajectory");
    for &at in &[0usize, 50, 150, 400, STEPS as usize - 1] {
        let s = &trajectory.samples[at];
        println!(
            "  t={:>5} ms  depth={:6.3} m  v={:+.3} m/s  thrust={:+.3}",
            s.time.as_nanos() / 1_000_000,
            s.true_depth,
            s.velocity,
            s.thrust,
        );
    }

    println!("\nconvergence");
    let err = (trajectory.final_depth() - SETPOINT).abs();
    println!(
        "  final depth : {:.3} m (setpoint {SETPOINT:.1} m, error {:.3} m)",
        trajectory.final_depth(),
        err,
    );
    let converged = err < 0.1;
    println!(
        "  status      : {}",
        if converged {
            "converged to setpoint"
        } else {
            "did NOT converge"
        }
    );

    println!("\nsafety decisions");
    println!(
        "  accepted {}, modified (clamped) {}, rejected {}",
        tally.accepted, tally.modified, tally.rejected
    );
    println!("  note: early large demands are clamped by the +/-0.8 thrust range");

    // Determinism: an independent second run must be byte-identical.
    println!("\ndeterminism");
    let first = digest(&trajectory);
    let (second_traj, _) = run_mission()?;
    let second = digest(&second_traj);
    println!("  digest run 1 : {first}");
    println!("  digest run 2 : {second}");
    let deterministic = first == second && trajectory == second_traj;
    println!(
        "  status       : {}",
        if deterministic {
            "verified (identical trajectories)"
        } else {
            "MISMATCH"
        }
    );

    if !converged {
        return Err("mission did not converge to setpoint".into());
    }
    if !deterministic {
        return Err("mission was not deterministic".into());
    }
    println!("\nmission complete.");
    Ok(())
}
