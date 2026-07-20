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
//! use neuradix_embedded_core::{
//!     AuthorityLease, CommandGate, EmbeddedComponent, Limits, PropulsionNode, NodeId,
//!     Outcome, SafeReason, Watchdog,
//! };
//! use neuradix_time::{ClockDomain, Duration, Timestamp};
//!
//! let t = |ns| Timestamp::new(ClockDomain::Monotonic, ns);
//! let gate = CommandGate::new(
//!     Limits::new(-1.0, 1.0, 0.5).unwrap(),
//!     AuthorityLease::until(t(10_000_000_000)), // lease valid for 10 s
//!     Watchdog::new(Duration::from_millis(100)), // 100 ms link timeout
//!     0.0, // safe output: zero thrust
//! );
//! let mut node = PropulsionNode::new(NodeId::new("thruster"), gate);
//!
//! // A fresh command is applied (first command: range-limited only).
//! assert_eq!(node.tick(t(0), Some(0.8)), 0.8);
//!
//! // 200 ms later with no command: the link is considered lost -> safe output.
//! let out = node.tick(t(200_000_000), None);
//! assert_eq!(out, 0.0);
//! assert!(node.in_safe_state());
//! assert_eq!(
//!     node.last_decision().unwrap().outcome,
//!     Outcome::SafeState(SafeReason::LinkLost),
//! );
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

pub use gate::{CommandGate, GateDecision, Limits, Outcome, SafeReason};
pub use health::HealthState;
pub use identity::{DeploymentId, NodeId};
pub use lease::AuthorityLease;
pub use node::{EmbeddedComponent, PropulsionNode};
pub use watchdog::Watchdog;
