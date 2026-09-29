//! Reference gateway data path for verified compact channels (WP-A02).
//!
//! ```text
//! bytes ─▶ FrameDecoder<F> (sync, length bound, CRC-32)
//!       ─▶ ChannelTable<N>::open (magic, version, manifest tag, compact ID, length)
//!       ─▶ generated decode(body, binding.wire_id)
//!       ─▶ typed handler  Handle<T>::handle(&binding, value)
//! ```
//!
//! - [`RoutingTable::new`] verifies the complete mapping **before** any traffic:
//!   the configured manifest and its digest, every expected route against the
//!   manifest and the expected contracts, every generated decoder registration,
//!   and the compiled generated board table. Any mismatch leaves no gateway.
//! - [`Gateway`] feeds received chunks through a fixed-size frame decoder and
//!   routes each verified frame. Handlers run only after every check passes.
//! - [`IdentityReport`] states the full manifest digest and the configured
//!   node, firmware/build and deployment identities with their sources.
//! - [`mod@reference`] is the checked-in two-route instance and a simulated
//!   producer; `examples/reference_gateway.rs` runs it.
//!
//! The existing transport, envelope, manifest and generated code are used
//! unchanged. The four-byte manifest tag detects a producer provisioned from a
//! different manifest; it is not proof of the peer's full manifest, identity or
//! authenticity. This crate is telemetry only: it carries no commands and does
//! not touch command authority, freshness or watchdogs. Serial discovery,
//! manifest exchange, authentication, reconnect and reboot handling, replay
//! protection and device sessions belong to WP-B06.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod decoder;
pub mod gateway;
pub mod identity;
pub mod reference;
pub mod routing;

pub use decoder::{GeneratedDecoder, Handle, Registration};
pub use gateway::{ChunkSummary, FrameOutcome, Gateway, Stats};
pub use identity::{Attributed, IdentityClaims, IdentityReport, IdentitySource};
pub use routing::{
    BoardTable, Delivered, ExpectedRoute, InitError, MAX_MANIFEST_BYTES, Rejection, Route,
    RoutingTable,
};
