//! The bounded in-process stream backend.
//!
//! The concrete queue (`VecDeque`) and synchronisation (`Mutex`) are private
//! implementation details; only the [`InProcessPublisher`] /
//! [`InProcessSubscriber`] handles and the neutral traits are public.

use std::collections::VecDeque;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{Arc, Mutex, MutexGuard};

use neuradix_contracts::{Delivery, OverflowPolicy};

use crate::{StreamPublisher, StreamSubscriber};

/// Configuration for a bounded stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    /// Maximum retained queue depth (always bounded; never zero).
    pub capacity: NonZeroUsize,
    /// Behaviour when the queue is full.
    pub overflow: OverflowPolicy,
}

impl StreamConfig {
    /// Construct a configuration from an explicit capacity and policy.
    pub fn new(capacity: NonZeroUsize, overflow: OverflowPolicy) -> Self {
        Self { capacity, overflow }
    }

    /// Derive a stream configuration from a contract's delivery policy.
    pub fn from_delivery(delivery: &Delivery) -> Self {
        Self {
            capacity: nonzero_u32_to_usize(delivery.capacity),
            overflow: delivery.overflow,
        }
    }
}

fn nonzero_u32_to_usize(value: NonZeroU32) -> NonZeroUsize {
    // A u32 always fits in usize on supported platforms (>= 32-bit) and the
    // value is non-zero, so the target is always valid.
    NonZeroUsize::new(value.get() as usize).expect("non-zero u32 is a non-zero usize")
}

/// The result of a publish into a bounded stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    /// The item was queued without discarding anything.
    Enqueued,
    /// The item was queued after evicting the oldest item (`drop-oldest`).
    DroppedOldest,
    /// The item was discarded because the queue was full (`drop-newest`).
    DroppedNewest,
    /// The item replaced a previously queued, not-yet-delivered item
    /// (`keep-latest`).
    Superseded,
    /// The item was refused because the queue was full (`reject`).
    Rejected,
}

/// Errors from stream operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// The stream has been closed by the producer.
    #[error("stream is closed")]
    Closed,
}

/// A snapshot of a stream's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamStats {
    /// Configured maximum depth.
    pub capacity: usize,
    /// Current queued depth.
    pub len: usize,
    /// Items that entered the queue.
    pub published: u64,
    /// Items handed to the subscriber.
    pub delivered: u64,
    /// Items lost to overflow (evicted, superseded or dropped-on-arrival).
    pub dropped: u64,
    /// Publish calls refused by the `reject` policy.
    pub rejected: u64,
    /// Whether the producer has closed the stream.
    pub closed: bool,
}

struct Shared<T> {
    queue: VecDeque<T>,
    capacity: usize,
    overflow: OverflowPolicy,
    published: u64,
    delivered: u64,
    dropped: u64,
    rejected: u64,
    closed: bool,
}

impl<T> Shared<T> {
    fn publish(&mut self, item: T) -> Result<PublishOutcome, StreamError> {
        if self.closed {
            return Err(StreamError::Closed);
        }

        // keep-latest retains only the single most recent item.
        if self.overflow == OverflowPolicy::KeepLatest {
            let superseded = self.queue.len();
            self.dropped += superseded as u64;
            self.queue.clear();
            self.queue.push_back(item);
            self.published += 1;
            return Ok(if superseded > 0 {
                PublishOutcome::Superseded
            } else {
                PublishOutcome::Enqueued
            });
        }

        if self.queue.len() < self.capacity {
            self.queue.push_back(item);
            self.published += 1;
            return Ok(PublishOutcome::Enqueued);
        }

        // Queue is full: apply the overflow policy.
        match self.overflow {
            OverflowPolicy::Reject => {
                self.rejected += 1;
                Ok(PublishOutcome::Rejected)
            }
            OverflowPolicy::DropNewest => {
                self.dropped += 1;
                Ok(PublishOutcome::DroppedNewest)
            }
            OverflowPolicy::DropOldest => {
                self.queue.pop_front();
                self.dropped += 1;
                self.queue.push_back(item);
                self.published += 1;
                Ok(PublishOutcome::DroppedOldest)
            }
            // Handled above; kept for exhaustiveness without panicking.
            OverflowPolicy::KeepLatest => {
                self.published += 1;
                Ok(PublishOutcome::Enqueued)
            }
        }
    }

    fn poll(&mut self) -> Option<T> {
        let item = self.queue.pop_front();
        if item.is_some() {
            self.delivered += 1;
        }
        item
    }

    fn stats(&self) -> StreamStats {
        StreamStats {
            capacity: self.capacity,
            len: self.queue.len(),
            published: self.published,
            delivered: self.delivered,
            dropped: self.dropped,
            rejected: self.rejected,
            closed: self.closed,
        }
    }
}

/// The producer handle for an in-process bounded stream.
pub struct InProcessPublisher<T> {
    shared: Arc<Mutex<Shared<T>>>,
}

/// The consumer handle for an in-process bounded stream.
pub struct InProcessSubscriber<T> {
    shared: Arc<Mutex<Shared<T>>>,
}

fn lock<T>(shared: &Arc<Mutex<Shared<T>>>) -> MutexGuard<'_, Shared<T>> {
    // Recover rather than panic if a holder panicked while holding the lock.
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl<T> StreamPublisher<T> for InProcessPublisher<T> {
    fn publish(&self, item: T) -> Result<PublishOutcome, StreamError> {
        lock(&self.shared).publish(item)
    }

    fn stats(&self) -> StreamStats {
        lock(&self.shared).stats()
    }

    fn config(&self) -> StreamConfig {
        let guard = lock(&self.shared);
        StreamConfig {
            capacity: NonZeroUsize::new(guard.capacity).expect("capacity is non-zero"),
            overflow: guard.overflow,
        }
    }

    fn close(&self) {
        lock(&self.shared).closed = true;
    }
}

impl<T> StreamSubscriber<T> for InProcessSubscriber<T> {
    fn poll(&self) -> Option<T> {
        lock(&self.shared).poll()
    }

    fn stats(&self) -> StreamStats {
        lock(&self.shared).stats()
    }

    fn is_closed(&self) -> bool {
        lock(&self.shared).closed
    }
}

/// Create a bounded in-process stream, returning a producer and consumer handle.
pub fn in_process<T>(config: StreamConfig) -> (InProcessPublisher<T>, InProcessSubscriber<T>) {
    let shared = Arc::new(Mutex::new(Shared {
        queue: VecDeque::with_capacity(config.capacity.get()),
        capacity: config.capacity.get(),
        overflow: config.overflow,
        published: 0,
        delivered: 0,
        dropped: 0,
        rejected: 0,
        closed: false,
    }));
    (
        InProcessPublisher {
            shared: Arc::clone(&shared),
        },
        InProcessSubscriber { shared },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StreamPublisher, StreamSubscriber};

    fn cfg(capacity: usize, overflow: OverflowPolicy) -> StreamConfig {
        StreamConfig::new(NonZeroUsize::new(capacity).unwrap(), overflow)
    }

    #[test]
    fn flows_in_order_when_not_full() {
        let (tx, rx) = in_process::<u32>(cfg(4, OverflowPolicy::Reject));
        for i in 0..3 {
            assert_eq!(tx.publish(i).unwrap(), PublishOutcome::Enqueued);
        }
        assert_eq!(rx.poll(), Some(0));
        assert_eq!(rx.poll(), Some(1));
        assert_eq!(rx.poll(), Some(2));
        assert_eq!(rx.poll(), None);
        assert_eq!(rx.stats().delivered, 3);
    }

    #[test]
    fn reject_refuses_when_full() {
        let (tx, rx) = in_process::<u32>(cfg(2, OverflowPolicy::Reject));
        assert_eq!(tx.publish(1).unwrap(), PublishOutcome::Enqueued);
        assert_eq!(tx.publish(2).unwrap(), PublishOutcome::Enqueued);
        assert_eq!(tx.publish(3).unwrap(), PublishOutcome::Rejected);
        let stats = tx.stats();
        assert_eq!(stats.rejected, 1);
        assert_eq!(stats.dropped, 0);
        assert_eq!(rx.poll(), Some(1)); // oldest retained
        assert_eq!(rx.poll(), Some(2));
        assert_eq!(rx.poll(), None);
    }

    #[test]
    fn drop_oldest_evicts_front() {
        let (tx, rx) = in_process::<u32>(cfg(2, OverflowPolicy::DropOldest));
        tx.publish(1).unwrap();
        tx.publish(2).unwrap();
        assert_eq!(tx.publish(3).unwrap(), PublishOutcome::DroppedOldest);
        assert_eq!(tx.stats().dropped, 1);
        assert_eq!(rx.poll(), Some(2)); // 1 was evicted
        assert_eq!(rx.poll(), Some(3));
        assert_eq!(rx.poll(), None);
    }

    #[test]
    fn drop_newest_discards_incoming() {
        let (tx, rx) = in_process::<u32>(cfg(2, OverflowPolicy::DropNewest));
        tx.publish(1).unwrap();
        tx.publish(2).unwrap();
        assert_eq!(tx.publish(3).unwrap(), PublishOutcome::DroppedNewest);
        assert_eq!(tx.stats().dropped, 1);
        assert_eq!(rx.poll(), Some(1)); // existing retained
        assert_eq!(rx.poll(), Some(2));
        assert_eq!(rx.poll(), None);
    }

    #[test]
    fn keep_latest_retains_only_the_newest() {
        let (tx, rx) = in_process::<u32>(cfg(8, OverflowPolicy::KeepLatest));
        assert_eq!(tx.publish(1).unwrap(), PublishOutcome::Enqueued);
        assert_eq!(tx.publish(2).unwrap(), PublishOutcome::Superseded);
        assert_eq!(tx.publish(3).unwrap(), PublishOutcome::Superseded);
        let stats = tx.stats();
        assert_eq!(stats.len, 1);
        assert_eq!(stats.dropped, 2);
        assert_eq!(rx.poll(), Some(3));
        assert_eq!(rx.poll(), None);
    }

    #[test]
    fn capacity_is_never_exceeded() {
        let (tx, rx) = in_process::<u32>(cfg(3, OverflowPolicy::DropOldest));
        for i in 0..100 {
            tx.publish(i).unwrap();
            assert!(tx.stats().len <= 3, "queue must never exceed its capacity");
        }
        let drained = std::iter::from_fn(|| rx.poll()).count();
        assert_eq!(drained, 3);
    }

    #[test]
    fn publishing_to_a_closed_stream_fails_but_draining_still_works() {
        let (tx, rx) = in_process::<u32>(cfg(4, OverflowPolicy::Reject));
        tx.publish(1).unwrap();
        tx.close();
        assert_eq!(tx.publish(2), Err(StreamError::Closed));
        assert!(rx.is_closed());
        assert_eq!(rx.poll(), Some(1)); // queued item still drains
        assert_eq!(rx.poll(), None);
    }

    #[test]
    fn config_from_contract_delivery_is_honoured() {
        let delivery = Delivery {
            capacity: NonZeroU32::new(2).unwrap(),
            overflow: OverflowPolicy::Reject,
        };
        let config = StreamConfig::from_delivery(&delivery);
        assert_eq!(config.capacity.get(), 2);
        assert_eq!(config.overflow, OverflowPolicy::Reject);
    }
}
