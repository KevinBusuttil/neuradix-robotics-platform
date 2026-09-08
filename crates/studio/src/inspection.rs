//! The inspection index and queries over a recording.

use std::collections::BTreeMap;

use neuradix_record::{RawRecord, Recording};

use crate::error::StudioError;
use crate::model::{ChannelSummary, DomainSpan, Timeline};
use crate::series::{ScalarDecoder, Series, SeriesPoint, SeriesStats};

/// A read-only inspection of a recording.
///
/// Built once over any [`Recording`] (native `.nrec` or MCAP), it answers the
/// queries a visualization layer needs — a timeline, per-channel statistics,
/// windowed record access and scalar series — without decoding payloads unless a
/// series is requested. All queries are pure functions of the recording, so
/// identical recordings yield identical results.
pub struct Inspection<'a> {
    recording: &'a dyn Recording,
    /// Per-channel record positions (indices into `records()`), sorted by time.
    by_channel: BTreeMap<u16, Vec<usize>>,
}

impl<'a> Inspection<'a> {
    /// Index a recording for querying.
    pub fn new(recording: &'a dyn Recording) -> Self {
        let records = recording.records();
        let mut by_channel: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
        for (pos, record) in records.iter().enumerate() {
            by_channel.entry(record.channel_id).or_default().push(pos);
        }
        // Sort each channel's positions by timestamp (stable, so equal
        // timestamps keep recorded order).
        for positions in by_channel.values_mut() {
            positions.sort_by_key(|&p| records[p].timestamp.as_nanos());
        }
        Self {
            recording,
            by_channel,
        }
    }

    /// The overall timeline: per-domain spans and per-channel summaries.
    pub fn timeline(&self) -> Timeline {
        let manifest = self.recording.manifest();

        // Channel order: manifest channels first, then any record-only channels.
        let mut order: Vec<u16> = manifest.channels.iter().map(|c| c.id).collect();
        for &id in self.by_channel.keys() {
            if !order.contains(&id) {
                order.push(id);
            }
        }

        let channels: Vec<ChannelSummary> = order
            .iter()
            .map(|&id| {
                let (name, schema_id, domain_hint) = match manifest.channel(id) {
                    Some(c) => (c.name.clone(), c.schema_id.clone(), c.clock_domain.clone()),
                    None => (format!("channel-{id}"), String::new(), String::new()),
                };
                self.summarize(id, name, schema_id, domain_hint)
            })
            .collect();

        Timeline {
            message_count: self.recording.records().len(),
            channel_count: channels.len(),
            domains: self.domain_spans(),
            channels,
        }
    }

    /// The records on `channel_id` whose timestamps fall in `[start, end]`
    /// (inclusive), in ascending time order.
    pub fn window(&self, channel_id: u16, start_nanos: i128, end_nanos: i128) -> Vec<&RawRecord> {
        let records = self.recording.records();
        let Some(positions) = self.by_channel.get(&channel_id) else {
            return Vec::new();
        };
        let lo = positions.partition_point(|&p| records[p].timestamp.as_nanos() < start_nanos);
        let hi = positions.partition_point(|&p| records[p].timestamp.as_nanos() <= end_nanos);
        // An inverted range (`start > end`) yields `lo > hi`; `get` returns
        // `None` for that rather than panicking on the slice.
        positions
            .get(lo..hi)
            .unwrap_or(&[])
            .iter()
            .map(|&p| &records[p])
            .collect()
    }

    /// The record on `channel_id` whose timestamp is closest to `at_nanos`.
    ///
    /// Ties resolve to the earlier record. Returns `None` if the channel has no
    /// records.
    pub fn nearest(&self, channel_id: u16, at_nanos: i128) -> Option<&RawRecord> {
        let records = self.recording.records();
        let positions = self.by_channel.get(&channel_id)?;
        if positions.is_empty() {
            return None;
        }
        let ip = positions.partition_point(|&p| records[p].timestamp.as_nanos() < at_nanos);

        let mut best: Option<(&RawRecord, u128)> = None;
        for cand in [ip.checked_sub(1), Some(ip)].into_iter().flatten() {
            if let Some(&p) = positions.get(cand) {
                let record = &records[p];
                let distance = abs_diff(record.timestamp.as_nanos(), at_nanos);
                if best.is_none_or(|(_, best_d)| distance < best_d) {
                    best = Some((record, distance));
                }
            }
        }
        best.map(|(record, _)| record)
    }

    /// Extract a scalar `field` from `channel_id` as a time-ordered series.
    ///
    /// Each payload is decoded with `decoder`; the named field is collected with
    /// its timestamp. Errors if the channel is unknown, a payload fails to
    /// decode, or the field is absent from a decoded payload.
    pub fn series<D: ScalarDecoder>(
        &self,
        channel_id: u16,
        field: &str,
        decoder: &D,
    ) -> Result<Series, StudioError> {
        let records = self.recording.records();
        let positions = self
            .by_channel
            .get(&channel_id)
            .ok_or(StudioError::UnknownChannel(channel_id))?;

        let mut points = Vec::with_capacity(positions.len());
        let mut domain = String::new();
        for &p in positions {
            let record = &records[p];
            if domain.is_empty() {
                domain = record.timestamp.domain().as_str().to_owned();
            }
            let scalars = decoder.decode(&record.payload)?;
            let value = scalars
                .iter()
                .find(|s| s.name == field)
                .map(|s| s.value)
                .ok_or_else(|| StudioError::FieldNotFound(field.to_owned()))?;
            points.push(SeriesPoint {
                nanos: record.timestamp.as_nanos(),
                value,
            });
        }

        let stats = SeriesStats::from_points(&points);
        Ok(Series {
            field: field.to_owned(),
            domain,
            points,
            stats,
        })
    }

    /// Build a [`ChannelSummary`] for one channel.
    fn summarize(
        &self,
        id: u16,
        name: String,
        schema_id: String,
        domain_hint: String,
    ) -> ChannelSummary {
        let records = self.recording.records();
        let positions = self.by_channel.get(&id).map(Vec::as_slice).unwrap_or(&[]);
        let count = positions.len();

        if count == 0 {
            return ChannelSummary {
                id,
                name,
                schema_id,
                clock_domain: domain_hint,
                count: 0,
                first_nanos: None,
                last_nanos: None,
                span_nanos: None,
                mean_period_nanos: None,
                rate_hz: None,
                min_payload: None,
                max_payload: None,
                total_payload: 0,
            };
        }

        // Positions are sorted by time, so first/last are the ends.
        let first_nanos = records[positions[0]].timestamp.as_nanos();
        let last_nanos = records[positions[count - 1]].timestamp.as_nanos();
        let clock_domain = records[positions[0]].timestamp.domain().as_str().to_owned();

        let mut min_payload = usize::MAX;
        let mut max_payload = 0usize;
        let mut total_payload = 0usize;
        for &p in positions {
            let len = records[p].payload.len();
            min_payload = min_payload.min(len);
            max_payload = max_payload.max(len);
            total_payload += len;
        }

        // Saturate rather than overflow on adversarial timestamps (the workspace
        // builds with overflow checks on, so a raw subtraction would panic).
        let span_nanos = last_nanos.saturating_sub(first_nanos);
        let mean_period_nanos = if count > 1 {
            Some(span_nanos / (count as i128 - 1))
        } else {
            None
        };
        let rate_hz = mean_period_nanos.and_then(|period| {
            if period > 0 {
                Some(1.0e9 / period as f64)
            } else {
                None
            }
        });

        ChannelSummary {
            id,
            name,
            schema_id,
            clock_domain,
            count,
            first_nanos: Some(first_nanos),
            last_nanos: Some(last_nanos),
            span_nanos: Some(span_nanos),
            mean_period_nanos,
            rate_hz,
            min_payload: Some(min_payload),
            max_payload: Some(max_payload),
            total_payload,
        }
    }

    /// Aggregate per-domain spans across all records.
    fn domain_spans(&self) -> Vec<DomainSpan> {
        // domain -> (start, end, count)
        let mut spans: BTreeMap<&'static str, (i128, i128, usize)> = BTreeMap::new();
        for record in self.recording.records() {
            let nanos = record.timestamp.as_nanos();
            let entry = spans
                .entry(record.timestamp.domain().as_str())
                .or_insert((nanos, nanos, 0));
            entry.0 = entry.0.min(nanos);
            entry.1 = entry.1.max(nanos);
            entry.2 += 1;
        }
        spans
            .into_iter()
            .map(|(domain, (start, end, count))| DomainSpan {
                domain: domain.to_owned(),
                start_nanos: start,
                end_nanos: end,
                duration_nanos: end.saturating_sub(start),
                message_count: count,
            })
            .collect()
    }
}

/// The absolute difference between two `i128` timestamps as a `u128`, without
/// overflow — the true range of `|a - b|` fits exactly in `u128`, whereas the
/// intermediate `a - b` can exceed `i128`.
fn abs_diff(a: i128, b: i128) -> u128 {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    hi.wrapping_sub(lo) as u128
}
