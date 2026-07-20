//! Embedded contract code generation (Embedded Profile WP1).
//!
//! Additional target projections over the same validated
//! [`neuradix_contracts::Contract`] the host Rust generator uses, so an MCU and
//! the host agree on the wire:
//!
//! - [`generate_nostd_rust`] — a `no_std` Rust payload struct with fixed
//!   little-endian `encode`/`decode`.
//! - [`generate_cpp`] — the equivalent Arduino/C++ header.
//! - [`golden_vectors`] — deterministic value → byte vectors, the cross-language
//!   conformance anchor: host Rust, `no_std` Rust and C++ must all reproduce
//!   them byte-for-byte. [`cpp_conformance_main`] emits a self-checking C++
//!   program against them.
//!
//! The wire is a fixed layout (each scalar field, in order, little-endian), so a
//! frame size is known at compile time and there is no ambiguity between
//! implementations. Variable-length fields are rejected.

pub mod cpp_gen;
pub mod error;
pub mod golden;
pub mod names;
pub mod rust_gen;
pub mod wire;

pub use cpp_gen::{GeneratedCpp, cpp_conformance_main, generate_cpp};
pub use error::CodegenError;
pub use golden::{GoldenField, GoldenSet, GoldenVector, golden_vectors};
pub use rust_gen::{GENERATOR_VERSION, GeneratedRust, generate_nostd_rust};
pub use wire::{ScalarValue, field_size};
