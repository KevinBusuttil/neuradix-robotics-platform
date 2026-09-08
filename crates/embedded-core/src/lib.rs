//! Executor-neutral component core for constrained MCUs (Embedded MCU tier).
//!
//! `neuradix-embedded-core` is the firmware-side twin of `neuradix-runtime`: the
//! static component model an actuator node runs without the full Linux runtime,
//! without a heap, and without committing to a particular embedded executor
//! (a host static loop, Embassy or RTIC bind later). It is `#![no_std]` and
//! allocation-free, and it reuses the **same** [`neuradix_time`] vocabulary as
//! the host so a node behaves identically in host simulation and on the board.
//!
//! # What it provides (Embedded Profile WP2)
//!
//! - [`NodeId`] / [`DeploymentId`] — provisioned identity.
//! - [`HealthState`] — the same health vocabulary as the host runtime.
//! - [`AuthorityLease`] — time-bounded permission to actuate.
//! - [`Watchdog`] — link-loss detection.
//! - [`CommandGate`] — the local command path: authority → link → validity →
//!   envelope (range + slew), with a **local safe output** whenever authority or
//!   the link is lost (§16.1, NRX-EMB-004). A wireless/serial link is never a
//!   safety channel; the safe response is local and time-driven.
//! - [`PropulsionNode`] — the reference AUV actuator node built from the above.
//!
//! # Example — link loss drives the local safe state
//!
//! ```
//! use neuradix_embedded_core::{AuthorityLease, Command, CommandMeta, CommandPolicy,
//!     CommandGate, Generation, Limits, Outcome, SafeReason, SessionConfig, SharedTimeline};
//! use neuradix_time::{ClockDomain, Duration, Timestamp};
//! let t = |n| Timestamp::new(ClockDomain::Monotonic, n);
//! // This fixture shares one clock and uses a simulation-only generation.
//! // Real startup requires a durable, non-reused generation and clock relationship.
//! let generation = Generation::new(1).unwrap();
//! let policy = CommandPolicy::new(SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
//!     Duration::from_secs(1), Duration::ZERO, Duration::from_millis(100)).unwrap();
//! let lease = AuthorityLease::new(1, 2, SessionConfig::new(generation, t(0), t(10_000_000_000), policy).unwrap());
//! let mut gate = CommandGate::new(Limits::with_slew_rate(-1.0, 1.0, 25.0).unwrap(), lease, 0.0).unwrap();
//! let input = Command { holder: 1, capability: 2, value: 0.8,
//!     meta: CommandMeta { generation, sequence: 0, source_at: t(0), deadline: t(1_000_000_000), timeline: 1 } };
//! assert_eq!(gate.evaluate(Some(input), t(0)).applied, 0.8);
//! assert_eq!(gate.evaluate(None, t(200_000_000)).outcome,
//!     Outcome::SafeState(SafeReason::WatchdogExpired));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod gate;
pub mod health;
pub mod identity;
pub mod lease;
pub mod node;
pub mod watchdog;

pub use gate::{Command, CommandGate, GateConfigError, GateDecision, Limits, Outcome, SafeReason};
pub use health::HealthState;
pub use identity::{DeploymentId, NodeId};
pub use lease::AuthorityLease;
pub use node::{EmbeddedComponent, PropulsionNode};
pub use watchdog::Watchdog;

/// Shared command validity configuration and metadata.
pub use neuradix_command_core::{
    CommandMeta, CommandPolicy, ConfigError as SessionError, Generation, SessionConfig,
    SharedTimeline,
};
