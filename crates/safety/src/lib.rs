//! # neuradix-safety
//!
//! Command authority, constraint enforcement and auditable safety decisions —
//! the path every actuator command traverses (§16).
//!
//! No ordinary component drives a safety-relevant actuator directly: a
//! [`CommandRequest`] is authorized against time-bounded [`AuthorityLease`]s and
//! passed through ordered [`Constraint`]s, producing a [`SafetyDecision`] that is
//! accepted, modified (clamped) or rejected (with a fail-safe value). Evaluation
//! is deterministic, so safety decisions replay identically (see RFC-0016).
//!
//! This increment implements the authority + constraint gate and its decision
//! evidence. Independent safety-island deployment, FDIR state machines and the
//! recorded command-lineage `explain` view are later increments (RFC-0005).
//!
//! ```
//! use neuradix_safety::{
//!     AuthorityLease, Capability, Constraint, Identity, CommandRequest, LeaseTable,
//!     Outcome, SafetyGate,
//! };
//! use neuradix_time::{ClockDomain, Timestamp};
//!
//! let holder = Identity::new("depth-controller");
//! let cap = Capability::new("propulsion/vertical-thrust");
//! let mut leases = LeaseTable::new();
//! leases.grant(AuthorityLease {
//!     holder: holder.clone(),
//!     capability: cap.clone(),
//!     priority: 10,
//!     issued: Timestamp::new(ClockDomain::Simulation, 0),
//!     expires: Timestamp::new(ClockDomain::Simulation, 1_000_000_000),
//!     envelope: None,
//! });
//! let mut gate = SafetyGate::new(leases, vec![Constraint::range("range", -4.0, 4.0).unwrap()], 0.0);
//!
//! let req = CommandRequest::new(holder, cap, 9.0, Timestamp::new(ClockDomain::Simulation, 10));
//! let decision = gate.evaluate(req);
//! assert_eq!(decision.outcome, Outcome::Modified); // clamped from 9.0 to 4.0
//! assert_eq!(decision.applied, 4.0);
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod authority;
pub mod constraint;
pub mod decision;
pub mod error;
pub mod fdir;
pub mod gate;
pub mod lineage;

pub use authority::{
    AuthorityDenial, AuthorityLease, Capability, CommandEnvelope, Identity, LeaseTable,
};
pub use constraint::Constraint;
pub use decision::{CommandRequest, Outcome, RejectReason, SafetyDecision};
pub use error::SafetyError;
pub use fdir::{FaultMode, FdirMonitor, FdirPolicy, FdirTransition};
pub use gate::SafetyGate;
pub use lineage::{CommandLineage, LINEAGE_CHANNEL, LineageOrigin};
