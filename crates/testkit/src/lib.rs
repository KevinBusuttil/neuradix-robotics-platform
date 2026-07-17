//! # neuradix-testkit
//!
//! Reusable test utilities shared across the Neuradix workspace so that test
//! logic is not duplicated crate by crate. It provides helpers for:
//!
//! * deterministic manual clocks ([`clock`]);
//! * contract golden-file comparison ([`golden`]);
//! * deterministic schema hashing ([`hashing`]);
//! * lifecycle transition assertions ([`lifecycle`]);
//! * bounded-stream behaviour ([`stream`]);
//! * structured CLI output assertions ([`cli_output`]).
//!
//! These helpers assert and panic on failure by design — that is their job in a
//! test — but they contain no `unsafe` code.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cli_output;
pub mod clock;
pub mod golden;
pub mod hashing;
pub mod lifecycle;
pub mod stream;
