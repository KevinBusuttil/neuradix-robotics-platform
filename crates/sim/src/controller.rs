//! The controller seam.
//!
//! A [`Controller`] closes the loop: given a measured depth at the current time,
//! it returns a thrust command. The seam is deliberately narrow so the
//! simulation core depends on nothing but time — a real control law, a
//! safety-gated law, or a recorded law can all implement it without pulling the
//! runtime, safety or transport crates into the simulation.

use neuradix_time::Timestamp;

/// Context for a single controller step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepContext {
    /// The current simulation time.
    pub now: Timestamp,
    /// The zero-based step index.
    pub step: u64,
}

/// A depth controller: maps a measured depth to a thrust command.
///
/// Implementations MUST be deterministic — the same sequence of
/// `(ctx, measured_depth)` calls must produce the same commands.
pub trait Controller {
    /// Produce a thrust command for `measured_depth` observed at `ctx.now`.
    fn command(&mut self, ctx: &StepContext, measured_depth: f64) -> f64;
}

/// A simple proportional depth controller driving toward a fixed setpoint.
///
/// The command is `gain * (setpoint - measured)`, clamped to `[-limit, limit]`.
/// Because positive thrust drives the vehicle downward, a vehicle shallower than
/// the setpoint (measured `<` setpoint) is commanded to descend.
#[derive(Debug, Clone, Copy)]
pub struct ProportionalController {
    /// The target depth (m).
    pub setpoint: f64,
    /// The proportional gain (command per metre of error).
    pub gain: f64,
    /// The symmetric command limit.
    pub limit: f64,
}

impl ProportionalController {
    /// Create a proportional controller.
    pub fn new(setpoint: f64, gain: f64, limit: f64) -> Self {
        Self {
            setpoint,
            gain,
            limit,
        }
    }
}

impl Controller for ProportionalController {
    fn command(&mut self, _ctx: &StepContext, measured_depth: f64) -> f64 {
        let error = self.setpoint - measured_depth;
        (self.gain * error).clamp(-self.limit, self.limit)
    }
}
