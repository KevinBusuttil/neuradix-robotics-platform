//! Bounded-stream test helpers.

use neuradix_transport_api::{StreamPublisher, StreamSubscriber};

/// Publish every item in `items`, returning the [`PublishOutcome`] for each.
///
/// [`PublishOutcome`]: neuradix_transport_api::PublishOutcome
///
/// # Panics
/// Panics if the stream is closed mid-publish.
pub fn publish_all<T, P>(
    publisher: &P,
    items: impl IntoIterator<Item = T>,
) -> Vec<neuradix_transport_api::PublishOutcome>
where
    P: StreamPublisher<T>,
{
    items
        .into_iter()
        .map(|item| publisher.publish(item).expect("stream should be open"))
        .collect()
}

/// Drain every currently-available item from `subscriber`.
pub fn drain_all<T, S>(subscriber: &S) -> Vec<T>
where
    S: StreamSubscriber<T>,
{
    let mut out = Vec::new();
    while let Some(item) = subscriber.poll() {
        out.push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroUsize;

    use neuradix_contracts::OverflowPolicy;
    use neuradix_transport_api::{StreamConfig, in_process};

    #[test]
    fn publish_and_drain_helpers_round_trip() {
        let cfg = StreamConfig::new(NonZeroUsize::new(4).unwrap(), OverflowPolicy::DropOldest);
        let (tx, rx) = in_process::<u32>(cfg);
        let outcomes = publish_all(&tx, [1, 2, 3]);
        assert_eq!(outcomes.len(), 3);
        assert_eq!(drain_all(&rx), vec![1, 2, 3]);
    }
}
