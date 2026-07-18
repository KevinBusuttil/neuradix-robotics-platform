//! Deployment graph compiler for the Neuradix Robotics Platform.
//!
//! This crate turns a declarative deployment manifest (`RobotDeployment`,
//! `deploy.neuradix.io/v1alpha1`) into a validated topology and reports policy
//! violations *before* anything is wired at runtime — the platform's "contracts
//! before connectivity" rule (§3.1, §28.2). Validation is pure and offline: it
//! never opens a transport, spawns a process, or reads a wall clock.
//!
//! # What it checks
//!
//! - **Structure** — required fields, known `apiVersion`/`kind`, duplicate
//!   node/component names.
//! - **Placement** — every component is placed on a declared node.
//! - **Contracts** — each connection carries a contract the producer *provides*
//!   and the consumer *requires*, and every requirement has a provider.
//! - **Runtime policy** — Python components (and Python producers feeding them)
//!   may not sit on the deterministic control path (§12.4 EXEC-007, §19.4).
//! - **Safety authority** — an actuator may only be commanded through a Safety
//!   component, and a deployment with actuators must declare a Safety authority
//!   (§16.1).
//! - **Topology** — the component graph must be acyclic.
//!
//! Structural and policy problems are surfaced as [`GraphIssue`]s in a
//! [`GraphReport`] rather than as hard errors, so a single pass reports every
//! problem at once. Only I/O and parse failures produce a [`GraphError`].
//!
//! Every report also carries a content-addressed [`deployment_identity`], so a
//! validated deployment can be pinned for production immutability (§28.4).
//!
//! # Example
//!
//! ```
//! use std::path::Path;
//! use neuradix_graph::from_yaml;
//!
//! let manifest = r#"
//! apiVersion: deploy.neuradix.io/v1alpha1
//! kind: RobotDeployment
//! metadata:
//!   name: demo
//! spec:
//!   nodes:
//!     - name: main
//!       target: linux-aarch64
//!   components:
//!     - name: planner
//!       node: main
//!       executionClass: interactive
//!       provides: [plan.v1]
//!     - name: safety
//!       node: main
//!       executionClass: deterministic
//!       role: safety
//!       requires: [plan.v1]
//!       provides: [cmd.v1]
//!     - name: thruster
//!       node: main
//!       executionClass: hard-real-time
//!       role: actuator
//!       requires: [cmd.v1]
//!   connections:
//!     - from: planner
//!       to: safety
//!       contract: plan.v1
//!     - from: safety
//!       to: thruster
//!       contract: cmd.v1
//! "#;
//!
//! let report = from_yaml(manifest, Path::new("demo.yaml")).unwrap();
//! assert!(report.is_valid());
//! assert!(report.identity.starts_with("sha256:"));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod identity;
pub mod model;
pub mod registry;
pub mod validate;

pub use error::GraphError;
pub use identity::deployment_identity;
pub use model::{
    Component, Connection, Deployment, ExecutionClass, Node, RawDeployment, Role, Runtime,
    SUPPORTED_API_VERSION, from_file, from_yaml_str,
};
pub use registry::{ContractEntry, ContractRegistry, RegistryError, Resolution};
pub use validate::{
    GraphIssue, GraphReport, ResolvedContract, Severity, from_yaml, load_file, validate,
    validate_with_registry,
};
