//! The inspection value model: timelines and channel summaries.
//!
//! These are plain data descriptions a visualization layer (Studio, an XR scene,
//! a CLI table) can render without touching the recording itself.

/// Per-clock-domain time span within a recording.
///
/// Domains are reported separately because cross-domain time arithmetic is not
/// meaningful — a recording may mix, e.g., `simulation` control samples and
/// `monotonic` command lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainSpan {
    /// The clock domain (canonical spelling).
    pub domain: String,
    /// Earliest timestamp in this domain (ns).
    pub start_nanos: i128,
    /// Latest timestamp in this domain (ns).
    pub end_nanos: i128,
    /// `end - start` (ns).
    pub duration_nanos: i128,
    /// Number of records in this domain.
    pub message_count: usize,
}

/// A per-channel statistical summary.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelSummary {
    /// Channel id.
    pub id: u16,
    /// Channel name.
    pub name: String,
    /// Content-addressed schema identity of the channel's payload.
    pub schema_id: String,
    /// Clock domain of the channel's timestamps.
    pub clock_domain: String,
    /// Number of records on the channel.
    pub count: usize,
    /// First record timestamp (ns), if any.
    pub first_nanos: Option<i128>,
    /// Last record timestamp (ns), if any.
    pub last_nanos: Option<i128>,
    /// `last - first` (ns), if at least one record.
    pub span_nanos: Option<i128>,
    /// Mean inter-sample period (ns), if at least two records.
    pub mean_period_nanos: Option<i128>,
    /// Effective sample rate (Hz), if a positive mean period exists.
    pub rate_hz: Option<f64>,
    /// Smallest payload size (bytes), if any.
    pub min_payload: Option<usize>,
    /// Largest payload size (bytes), if any.
    pub max_payload: Option<usize>,
    /// Total payload bytes across the channel.
    pub total_payload: usize,
}

/// The overall timeline of a recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// Total number of records.
    pub message_count: usize,
    /// Number of channels described.
    pub channel_count: usize,
    /// Per-domain spans, sorted by domain name.
    pub domains: Vec<DomainSpan>,
    /// Per-channel summaries, in manifest order then record-only channels.
    pub channels: Vec<ChannelSummary>,
}
