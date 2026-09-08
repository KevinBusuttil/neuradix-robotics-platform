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
//! Both host and embedded gates share allocation-free command validity checks.
//! FDIR and recorded command-lineage inspection are available; independent
//! safety-island deployment remains future work (RFC-0005).
//!
//! ```
//! use neuradix_safety::{AuthorityLease, Capability, CommandMeta, CommandPolicy,
//!     CommandRequest, Constraint, Generation, Identity, LeaseTable, Outcome,
//!     SafetyGate, SessionConfig, SharedTimeline};
//! use neuradix_time::{ClockDomain, Duration, Timestamp};
//! let t = |n| Timestamp::new(ClockDomain::Simulation, n);
//! // Fixed identifiers are for this isolated example. Live startup must reserve
//! // a non-reused generation durably BEFORE enabling command ingress.
//! let generation = Generation::new(1).unwrap();
//! let policy = CommandPolicy::new(SharedTimeline::new(1, ClockDomain::Simulation).unwrap(),
//!     Duration::from_millis(100), Duration::ZERO, Duration::from_millis(100)).unwrap();
//! let config = SessionConfig::new(generation, t(0), t(1_000_000_000), policy).unwrap();
//! let holder = Identity::new("controller");
//! let cap = Capability::new("thrust");
//! let mut leases = LeaseTable::new();
//! leases.grant(AuthorityLease::new(holder.clone(), cap.clone(), config, None)).unwrap();
//! let mut gate = SafetyGate::new(leases, vec![Constraint::range("range", -4.0, 4.0).unwrap()], 0.0).unwrap();
//! let request = CommandRequest::new(holder, cap, 9.0, CommandMeta { generation,
//!     sequence: 0, source_at: t(10), deadline: t(100), timeline: 1 });
//! let decision = gate.evaluate(Some(request), t(10));
//! assert_eq!(decision.outcome, Outcome::Modified);
//! assert_eq!(decision.applied, 4.0);
//! // Schedule idle ticks too: deadlines must expire without a new command.
//! assert!(gate.evaluate(None, t(100)).is_rejected());
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
pub use lineage::{CommandLineage, CommandMetadata, LINEAGE_CHANNEL, LineageOrigin};

/// Shared command validity configuration and metadata.
pub use neuradix_command_core::{CommandMeta, CommandPolicy, ConfigError as SessionError, Generation, SessionConfig, SharedTimeline};
