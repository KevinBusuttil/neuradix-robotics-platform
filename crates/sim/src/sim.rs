//! The closed-loop simulation driver.
//!
//! Each fixed step observes the plant through the sensor, asks the controller
//! for a command, integrates the plant under that command, and advances an
//! injected [`ManualClock`]. The recorded [`Trajectory`] is a pure function of
//! the initial state, parameters and controller, so two identical runs produce
//! byte-identical trajectories — the property the deterministic-replay story
//! (RFC-0003) depends on.

use neuradix_time::{Clock, Duration, ManualClock, Timestamp};

use crate::controller::{Controller, StepContext};
use crate::error::SimError;
use crate::plant::{DepthPlant, PlantState};
use crate::sensor::DepthSensor;

/// One recorded step of a simulation run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrajectorySample {
    /// The zero-based step index.
    pub step: u64,
    /// The time at which the state was observed and the command issued.
    pub time: Timestamp,
    /// The plant's true depth at `time` (m).
    pub true_depth: f64,
    /// The sensor's measured depth at `time` (m).
    pub measured_depth: f64,
    /// The plant's vertical velocity at `time` (m/s, positive downward).
    pub velocity: f64,
    /// The thrust command applied over the step following `time`.
    pub thrust: f64,
}

/// A full simulation run: the ordered samples plus the final plant state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trajectory {
    /// The per-step samples in time order.
    pub samples: Vec<TrajectorySample>,
    /// The plant state after the final step.
    pub final_state: PlantState,
}

impl Trajectory {
    /// The number of steps recorded.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no steps were recorded.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The final true depth (m) — the depth after the last step.
    pub fn final_depth(&self) -> f64 {
        self.final_state.depth
    }
}

impl Default for PlantState {
    fn default() -> Self {
        Self::at_surface()
    }
}

/// A closed-loop depth simulation over a plant, sensor and controller.
#[derive(Debug)]
pub struct Simulation<C: Controller> {
    clock: ManualClock,
    plant: DepthPlant,
    sensor: DepthSensor,
    controller: C,
    dt: Duration,
    dt_secs: f64,
    step: u64,
}

impl<C: Controller> Simulation<C> {
    /// Build a simulation starting at `start`, stepping by `dt`.
    ///
    /// `dt` must be strictly positive. The clock's domain is taken from `start`,
    /// so every trajectory timestamp carries it.
    pub fn new(
        start: Timestamp,
        dt: Duration,
        plant: DepthPlant,
        sensor: DepthSensor,
        controller: C,
    ) -> Result<Self, SimError> {
        if dt.as_nanos() <= 0 {
            return Err(SimError::NonPositiveStep);
        }
        Ok(Self {
            clock: ManualClock::new(start),
            plant,
            sensor,
            controller,
            dt,
            dt_secs: dt.as_secs_f64(),
            step: 0,
        })
    }

    /// The current plant state.
    pub fn state(&self) -> PlantState {
        self.plant.state()
    }

    /// The controller, for inspecting its state after a run.
    pub fn controller(&self) -> &C {
        &self.controller
    }

    /// The current simulation time.
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Run `steps` closed-loop steps, returning the recorded trajectory.
    ///
    /// May be called repeatedly to continue a run; step indices and time
    /// continue from where the previous call left off.
    pub fn run(&mut self, steps: u64) -> Result<Trajectory, SimError> {
        let mut samples = Vec::with_capacity(steps as usize);
        for _ in 0..steps {
            let now = self.clock.now();
            let ctx = StepContext {
                now,
                step: self.step,
            };

            // Observe the pre-step state, decide, then integrate.
            let observed = self.plant.state();
            let measured = self.sensor.observe(observed.depth);
            let thrust = self.controller.command(&ctx, measured);
            self.plant.step(thrust, self.dt_secs);

            samples.push(TrajectorySample {
                step: self.step,
                time: now,
                true_depth: observed.depth,
                measured_depth: measured,
                velocity: observed.velocity,
                thrust,
            });

            // Advance the injected clock by exactly one fixed step.
            let next = now.checked_add(self.dt)?;
            self.clock.set(next)?;
            self.step += 1;
        }

        Ok(Trajectory {
            samples,
            final_state: self.plant.state(),
        })
    }
}
