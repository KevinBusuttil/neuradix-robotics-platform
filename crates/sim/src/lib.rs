//! Deterministic vehicle simulation for the Neuradix AUV vertical slice.
//!
//! This crate closes the control loop the rest of the platform proves in the
//! open: a [`DepthPlant`] with simple, fixed-step vertical dynamics, a
//! deterministic [`DepthSensor`], and a [`Simulation`] driver that steps
//! sensor → controller → plant under an injected clock. It lets the vertical
//! slice run against a *simulated vehicle* rather than a canned input sequence,
//! while preserving the platform's core property: a run is a pure function of
//! its parameters, initial state and controller, so two identical runs produce
//! byte-identical [`Trajectory`] output (RFC-0003).
//!
//! The [`Controller`] seam is deliberately narrow (measured depth in, thrust
//! out) so the simulation core depends only on `neuradix-time`. A real control
//! law — including one routed through the safety gate — implements the trait
//! without dragging the runtime, safety or transport crates into the model.
//!
//! # Example
//!
//! ```
//! use neuradix_sim::{
//!     DepthPlant, DepthSensor, PlantParams, PlantState, ProportionalController, Simulation,
//! };
//! use neuradix_time::{ClockDomain, Duration, Timestamp};
//!
//! let plant = DepthPlant::new(PlantParams::default(), PlantState::at_surface()).unwrap();
//! let controller = ProportionalController::new(5.0, 0.6, 1.0);
//! let mut sim = Simulation::new(
//!     Timestamp::new(ClockDomain::Simulation, 0),
//!     Duration::from_millis(20),
//!     plant,
//!     DepthSensor::ideal(),
//!     controller,
//! )
//! .unwrap();
//!
//! let trajectory = sim.run(500).unwrap();
//! // The proportional controller drives the vehicle toward the 5 m setpoint.
//! assert!((trajectory.final_depth() - 5.0).abs() < 0.1);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod controller;
pub mod error;
pub mod plant;
pub mod sensor;
pub mod sim;

pub use controller::{Controller, ProportionalController, StepContext};
pub use error::SimError;
pub use plant::{DepthPlant, PlantParams, PlantState};
pub use sensor::{DepthSensor, SensorParams};
pub use sim::{Simulation, Trajectory, TrajectorySample};
