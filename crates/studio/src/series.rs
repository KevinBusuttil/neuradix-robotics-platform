//! Scalar time-series extraction.
//!
//! Payloads are opaque bytes, so turning them into plottable numbers requires a
//! caller-supplied [`ScalarDecoder`]. This keeps the inspection layer neutral:
//! it never assumes a wire encoding, and any decoder (a fixed-layout codec, a
//! command-lineage reader, a future Protobuf decoder) plugs into the same
//! [`Inspection::series`](crate::Inspection::series) query.

use crate::error::StudioError;

/// A named scalar extracted from a payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarSample {
    /// The field name.
    pub name: String,
    /// The field value.
    pub value: f64,
}

impl ScalarSample {
    /// Construct a scalar sample.
    pub fn new(name: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

/// Decodes a payload into its named scalar fields.
///
/// Implementations MUST be deterministic. A decoder that cannot interpret a
/// payload returns [`StudioError::Decode`].
pub trait ScalarDecoder {
    /// Decode all scalar fields carried by `payload`.
    fn decode(&self, payload: &[u8]) -> Result<Vec<ScalarSample>, StudioError>;
}

/// One point of a scalar series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesPoint {
    /// Timestamp (ns) within the channel's clock domain.
    pub nanos: i128,
    /// The field value at that time.
    pub value: f64,
}

/// Aggregate statistics over a scalar series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesStats {
    /// Number of points.
    pub count: usize,
    /// Minimum value.
    pub min: f64,
    /// Maximum value.
    pub max: f64,
    /// Arithmetic mean.
    pub mean: f64,
    /// First value (earliest time).
    pub first: f64,
    /// Last value (latest time).
    pub last: f64,
}

impl SeriesStats {
    /// Compute stats over ordered points, or `None` if empty.
    pub fn from_points(points: &[SeriesPoint]) -> Option<Self> {
        let (first, last) = (points.first()?, points.last()?);
        let mut min = first.value;
        let mut max = first.value;
        let mut sum = 0.0;
        for p in points {
            if p.value < min {
                min = p.value;
            }
            if p.value > max {
                max = p.value;
            }
            sum += p.value;
        }
        Some(Self {
            count: points.len(),
            min,
            max,
            mean: sum / points.len() as f64,
            first: first.value,
            last: last.value,
        })
    }
}

/// A scalar series extracted from one channel: the plottable form.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// The extracted field name.
    pub field: String,
    /// The channel's clock domain.
    pub domain: String,
    /// The points, in ascending time order.
    pub points: Vec<SeriesPoint>,
    /// Aggregate statistics, or `None` if the series is empty.
    pub stats: Option<SeriesStats>,
}
