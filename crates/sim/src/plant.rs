//! The vertical-depth plant model.
//!
//! A deterministic point-mass model of an AUV's vertical axis. Depth is metres,
//! **positive downward** (`0.0` is the surface); velocity is metres per second,
//! positive downward; accelerations are metres per second squared. Integration
//! is fixed-step semi-implicit Euler, so a trajectory is a pure function of the
//! parameters, the initial state and the command sequence — no randomness and no
//! ambient time.

use crate::error::SimError;

/// Physical parameters of the depth plant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantParams {
    /// Downward acceleration produced by a unit thrust command
    /// (m/s² per unit command). Must be finite and `>= 0`.
    pub thrust_accel: f64,
    /// Net buoyancy expressed as an *upward* acceleration (m/s²): a positive
    /// value makes an uncommanded vehicle rise toward the surface. Zero models a
    /// neutrally buoyant vehicle. Must be finite.
    pub buoyancy_accel: f64,
    /// Linear drag coefficient (1/s); the drag acceleration is `-drag * velocity`
    /// and always opposes motion. Must be finite and `>= 0`.
    pub drag: f64,
    /// The deepest representable depth (m); the trajectory floor. Must be finite
    /// and `> 0`.
    pub max_depth: f64,
}

impl Default for PlantParams {
    /// A small, slightly buoyant AUV that sinks under command and drifts up when
    /// released.
    fn default() -> Self {
        Self {
            thrust_accel: 2.0,
            buoyancy_accel: 0.05,
            drag: 0.8,
            max_depth: 100.0,
        }
    }
}

impl PlantParams {
    /// Validate the parameters, returning a typed error for the first problem.
    pub fn validate(&self) -> Result<(), SimError> {
        check_finite("thrust_accel", self.thrust_accel)?;
        non_negative("thrust_accel", self.thrust_accel)?;
        check_finite("buoyancy_accel", self.buoyancy_accel)?;
        check_finite("drag", self.drag)?;
        non_negative("drag", self.drag)?;
        check_finite("max_depth", self.max_depth)?;
        if self.max_depth <= 0.0 {
            return Err(SimError::InvalidParameter {
                name: "max_depth",
                reason: "must be strictly positive".to_owned(),
            });
        }
        Ok(())
    }
}

/// The instantaneous kinematic state of the plant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantState {
    /// Depth below the surface (m), always in `[0, max_depth]`.
    pub depth: f64,
    /// Vertical velocity (m/s), positive downward.
    pub velocity: f64,
}

impl PlantState {
    /// A state at rest on the surface.
    pub const fn at_surface() -> Self {
        Self {
            depth: 0.0,
            velocity: 0.0,
        }
    }

    /// A state with the given depth and velocity.
    pub const fn new(depth: f64, velocity: f64) -> Self {
        Self { depth, velocity }
    }
}

/// A depth plant: parameters plus the current state.
#[derive(Debug, Clone)]
pub struct DepthPlant {
    params: PlantParams,
    state: PlantState,
}

impl DepthPlant {
    /// Create a plant with validated parameters and an initial state.
    ///
    /// The initial depth is clamped into `[0, max_depth]` so the plant always
    /// starts inside its physical envelope.
    pub fn new(params: PlantParams, initial: PlantState) -> Result<Self, SimError> {
        params.validate()?;
        if !initial.depth.is_finite() || !initial.velocity.is_finite() {
            return Err(SimError::InvalidParameter {
                name: "initial",
                reason: "depth and velocity must be finite".to_owned(),
            });
        }
        let depth = initial.depth.clamp(0.0, params.max_depth);
        Ok(Self {
            params,
            state: PlantState {
                depth,
                velocity: initial.velocity,
            },
        })
    }

    /// The current state.
    pub fn state(&self) -> PlantState {
        self.state
    }

    /// The plant parameters.
    pub fn params(&self) -> &PlantParams {
        &self.params
    }

    /// Integrate one fixed step of length `dt_secs` seconds under a `thrust`
    /// command (unitless; the caller — typically the safety gate — is
    /// responsible for clamping it to the actuator envelope), returning the new
    /// state.
    ///
    /// Uses semi-implicit Euler (velocity updated first, then position), and
    /// clamps depth to `[0, max_depth]`, zeroing velocity at a boundary so the
    /// vehicle neither breaches the surface nor tunnels through the floor.
    pub fn step(&mut self, thrust: f64, dt_secs: f64) -> PlantState {
        let accel = self.params.thrust_accel * thrust
            - self.params.buoyancy_accel
            - self.params.drag * self.state.velocity;

        let mut velocity = self.state.velocity + accel * dt_secs;
        let mut depth = self.state.depth + velocity * dt_secs;

        if depth <= 0.0 {
            depth = 0.0;
            if velocity < 0.0 {
                velocity = 0.0;
            }
        } else if depth >= self.params.max_depth {
            depth = self.params.max_depth;
            if velocity > 0.0 {
                velocity = 0.0;
            }
        }

        self.state = PlantState { depth, velocity };
        self.state
    }
}

fn check_finite(name: &'static str, value: f64) -> Result<(), SimError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(SimError::InvalidParameter {
            name,
            reason: "must be a finite number".to_owned(),
        })
    }
}

fn non_negative(name: &'static str, value: f64) -> Result<(), SimError> {
    if value >= 0.0 {
        Ok(())
    } else {
        Err(SimError::InvalidParameter {
            name,
            reason: "must not be negative".to_owned(),
        })
    }
}
