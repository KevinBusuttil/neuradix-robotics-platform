//! # neuradix-contracts
//!
//! The Neuradix contract model: parsing, validation, deterministic
//! canonicalization, content-addressed schema identity and Rust code
//! generation.
//!
//! This crate is the dependency root of the platform: it depends on no other
//! Neuradix crate. The runtime, transport layer, CLI and code generators all
//! build on the validated [`Contract`] model defined here.
//!
//! ## Pipeline
//!
//! ```text
//! authored YAML  ->  raw document  ->  validated Contract
//!                                          |-> schema identity (sha256:...)
//!                                          |-> generated Rust
//! ```
//!
//! ## Example
//!
//! ```
//! use neuradix_contracts::{schema_identity, validate};
//! use std::path::Path;
//!
//! let yaml = r#"
//! apiVersion: contracts.neuradix.io/v1alpha1
//! kind: StreamContract
//! metadata:
//!   namespace: io.neuradix.navigation
//!   name: vehicle-depth
//!   version: 1.0.0
//! spec:
//!   description: Vehicle depth below the configured water reference
//!   payload:
//!     type: object
//!     fields:
//!       depth: { type: float64, unit: m }
//!   semantics:
//!     frame: vehicle/base
//!     clockDomain: monotonic
//!     authoritativeTimestamp: measurement
//!     maximumAge: 100ms
//!   delivery:
//!     capacity: 8
//!     overflow: keep-latest
//! "#;
//!
//! let contract = validate::from_yaml_str(yaml, Path::new("<inline>")).unwrap();
//! assert_eq!(contract.metadata.name, "vehicle-depth");
//! assert!(schema_identity(&contract).as_str().starts_with("sha256:"));
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod canonical;
pub mod error;
pub mod generate;
pub mod model;
pub mod parse;
pub mod validate;

pub use canonical::{SchemaId, canonical_bytes, schema_identity};
pub use error::{ContractError, Result, ValidationIssue};
pub use generate::{GENERATOR_VERSION, GeneratedRust, generate_rust};
pub use model::{
    ClockDomainRef, Contract, ContractKind, Delivery, Duration, Field, Metadata, OverflowPolicy,
    Payload, PrimitiveType, SUPPORTED_API_VERSION, Semantics, Spec,
};
pub use validate::load_file;
