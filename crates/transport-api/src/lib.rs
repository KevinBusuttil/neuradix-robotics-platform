//! # neuradix-transport-api
//!
//! The transport-neutral data-plane interface for Neuradix, plus the first
//! backend: a bounded, in-process stream.
//!
//! Component-domain code programs against the [`StreamPublisher`] and
//! [`StreamSubscriber`] traits, never against a concrete queue or channel type.
//! The in-process backend is one implementation; shared-memory, Zenoh, CAN and
//! serial backends can be added later without changing component code, because
//! none of those backend types appear in this crate's public API.
//!
//! Overflow behaviour is described by [`neuradix_contracts::OverflowPolicy`], so
//! the authored contract policy and the runtime behaviour cannot drift apart.
//! Per-policy semantics are specified in
//! `docs/rfcs/RFC-0004-Transport-Neutral-Data-Plane.md`; the summary is:
//!
//! | Policy         | When the queue is full                                   |
//! |----------------|----------------------------------------------------------|
//! | `reject`       | refuse the incoming item (counted in `rejected`)         |
//! | `drop-oldest`  | evict the oldest queued item, enqueue the new one        |
//! | `drop-newest`  | drop the incoming item (counted in `dropped`)            |
//! | `keep-latest`  | retain only the single most recent item (depth ≤ 1)      |
//!
//! ```
//! use std::num::NonZeroUsize;
//! use neuradix_contracts::OverflowPolicy;
//! use neuradix_transport_api::{StreamConfig, StreamPublisher, StreamSubscriber, in_process};
//!
//! let cfg = StreamConfig::new(NonZeroUsize::new(2).unwrap(), OverflowPolicy::DropOldest);
//! let (tx, rx) = in_process::<u32>(cfg);
//! tx.publish(1).unwrap();
//! tx.publish(2).unwrap();
//! tx.publish(3).unwrap(); // full: evicts `1`
//! assert_eq!(rx.poll(), Some(2));
//! assert_eq!(rx.poll(), Some(3));
//! assert_eq!(rx.stats().dropped, 1);
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod stream;

pub use stream::{
    InProcessPublisher, InProcessSubscriber, PublishOutcome, StreamConfig, StreamError,
    StreamStats, in_process,
};

/// The producer half of a transport-neutral stream.
pub trait StreamPublisher<T> {
    /// Publish an item. Returns the [`PublishOutcome`] describing how the
    /// bounded queue handled it, or [`StreamError::Closed`] if the stream is
    /// closed.
    fn publish(&self, item: T) -> Result<PublishOutcome, StreamError>;

    /// A snapshot of stream statistics.
    fn stats(&self) -> StreamStats;

    /// The configured capacity and overflow policy.
    fn config(&self) -> StreamConfig;

    /// Close the stream. Subsequent publishes fail; queued items can still be
    /// drained by the subscriber.
    fn close(&self);
}

/// The consumer half of a transport-neutral stream.
pub trait StreamSubscriber<T> {
    /// Take the next available item, or `None` if the queue is currently empty.
    fn poll(&self) -> Option<T>;

    /// A snapshot of stream statistics.
    fn stats(&self) -> StreamStats;

    /// Whether the producer side has been closed.
    fn is_closed(&self) -> bool;
}
