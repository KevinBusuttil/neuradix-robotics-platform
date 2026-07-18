//! # neuradix-runtime
//!
//! A minimal component and lifecycle model: stable component identity, an
//! explicit and auditable lifecycle state machine, an execution-class
//! classification, a structured health model and typed errors.
//!
//! This increment provides only enough runtime structure to build and validate
//! components and run the minimal depth-stream example cleanly. There is no
//! distributed supervisor, scheduler or executor yet; those are later
//! increments. The public API is transport- and executor-neutral.
//!
//! ```
//! use neuradix_runtime::{Lifecycle, LifecycleState};
//! use neuradix_time::{ClockDomain, ManualClock, Timestamp, Clock};
//!
//! let clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
//! let mut lifecycle = Lifecycle::new();
//! lifecycle.transition(LifecycleState::Configured, "config ok", "test", clock.now()).unwrap();
//! lifecycle.transition(LifecycleState::Inactive, "resolved", "test", clock.now()).unwrap();
//! assert_eq!(lifecycle.current(), LifecycleState::Inactive);
//! // An illegal jump is rejected:
//! assert!(lifecycle.transition(LifecycleState::Stopped, "x", "test", clock.now()).is_err());
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod component;
pub mod error;
pub mod executor;
pub mod health;
pub mod id;
pub mod lifecycle;

pub use component::{Component, ComponentManifest, ExecutionClass};
pub use error::{ComponentError, LifecycleError};
pub use executor::{Processor, TickContext, run_lockstep};
pub use health::{HealthReport, HealthState};
pub use id::ComponentId;
pub use lifecycle::{Lifecycle, LifecycleState, TransitionRecord};
