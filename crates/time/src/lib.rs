//! # neuradix-time
//!
//! Explicit time semantics for Neuradix: clock domains, domain-tagged
//! timestamps, a signed nanosecond [`Duration`], and an injectable [`Clock`]
//! abstraction with a non-deterministic [`SystemClock`] and a deterministic
//! [`ManualClock`].
//!
//! Two rules are structural rather than advisory:
//!
//! 1. A [`Timestamp`] always carries its [`ClockDomain`]; arithmetic across
//!    domains is a typed [`TimeError`], never a silent bug.
//! 2. Deterministic and replayable logic takes time from an injected [`Clock`].
//!    The [`ManualClock`] advances with no sleeping and no ambient clock access.
//!
//! ```
//! use neuradix_time::{Clock, ClockDomain, Duration, ManualClock, Timestamp};
//!
//! let clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
//! let t0 = clock.now();
//! clock.advance(Duration::from_millis(50)).unwrap();
//! let t1 = clock.now();
//! assert_eq!(t1.duration_since(t0).unwrap(), Duration::from_millis(50));
//! ```
//! ## `no_std`
//!
//! This crate is `no_std`-compatible: with `default-features = false` it depends
//! only on `core`, so the same [`Timestamp`], [`Duration`] and [`ClockDomain`]
//! vocabulary is shared by the Linux runtime and by constrained MCU firmware
//! (`neuradix-embedded-core`). The `std` feature (default) additionally provides
//! the ambient [`SystemClock`]. The deterministic [`ManualClock`] and all core
//! types are available in both modes.
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clock;
pub mod domain;
pub mod duration;
pub mod error;
pub mod timestamp;

#[cfg(feature = "std")]
pub use clock::SystemClock;
pub use clock::{Clock, ControllableClock, ManualClock};
pub use domain::ClockDomain;
pub use duration::Duration;
pub use error::TimeError;
pub use timestamp::Timestamp;
