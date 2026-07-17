//! The validated, in-memory contract model.
//!
//! This is the semantic model produced after a raw authored document (see
//! [`crate::parse`]) has passed validation (see [`crate::validate`]). Only
//! valid contracts can be represented by these types; the code generator and
//! the schema-identity hash operate on this model, never on raw YAML.

use std::num::NonZeroU32;

/// The `apiVersion` accepted by this implementation.
///
/// The v0.5 functional specification (§10.3) shows an illustrative
/// `neuradix.io/v1alpha1` example; the concrete authored format for this
/// foundation increment is `contracts.neuradix.io/v1alpha1`. The divergence is
/// recorded in `docs/rfcs/RFC-0002-Contract-Format-and-Code-Generation.md`.
pub const SUPPORTED_API_VERSION: &str = "contracts.neuradix.io/v1alpha1";

/// A fully validated Neuradix contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    /// The API version of the authored document.
    pub api_version: String,
    /// The kind of contract. Only [`ContractKind::StreamContract`] is supported
    /// in this increment.
    pub kind: ContractKind,
    /// Identity metadata.
    pub metadata: Metadata,
    /// The contract body.
    pub spec: Spec,
}

impl Contract {
    /// The `namespace/name` identifier used in diagnostics.
    pub fn identifier(&self) -> String {
        format!("{}/{}", self.metadata.namespace, self.metadata.name)
    }
}

/// Supported contract kinds.
///
/// Neuradix defines six communication primitives (§9). This increment
/// implements only the `StreamContract`; the remaining kinds are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractKind {
    /// A sequence of timestamped samples or frames (§9.1).
    StreamContract,
}

impl ContractKind {
    /// The canonical string spelling of this kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            ContractKind::StreamContract => "StreamContract",
        }
    }
}

/// Identity metadata for a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// Reverse-domain namespace, e.g. `io.neuradix.navigation`.
    pub namespace: String,
    /// Short contract name, e.g. `vehicle-depth`.
    pub name: String,
    /// Semantic version of the contract.
    pub version: semver::Version,
}

/// The body of a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// Optional human documentation. Non-normative: it does NOT affect schema
    /// identity.
    pub description: Option<String>,
    /// The payload shape.
    pub payload: Payload,
    /// Semantic metadata (frame, clock domain, timestamp, validity).
    pub semantics: Semantics,
    /// Delivery / queueing policy.
    pub delivery: Delivery,
}

/// A payload described as an ordered set of scalar fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// Fields in authored declaration order. Order is preserved for code
    /// generation but does NOT affect schema identity (fields are sorted by
    /// name before hashing).
    pub fields: Vec<Field>,
}

/// A single scalar payload field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Field name (a valid lower_snake or lowerCamel identifier fragment).
    pub name: String,
    /// The scalar primitive type.
    pub ty: PrimitiveType,
    /// Optional physical unit metadata, preserved verbatim (e.g. `m`).
    pub unit: Option<String>,
}

/// The scalar primitive types understood by this increment.
///
/// This is deliberately a narrow subset (§6.3 of the task brief). Nested
/// objects, arrays, maps, enums and large-buffer types are not yet supported
/// and are reported as clear validation errors rather than silently accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    /// 64-bit IEEE-754 float (`f64`).
    Float64,
    /// 32-bit IEEE-754 float (`f32`).
    Float32,
    /// Signed 32-bit integer (`i32`).
    Int32,
    /// Signed 64-bit integer (`i64`).
    Int64,
    /// Unsigned 32-bit integer (`u32`).
    Uint32,
    /// Unsigned 64-bit integer (`u64`).
    Uint64,
    /// Boolean (`bool`).
    Bool,
    /// UTF-8 string (`String`).
    Str,
}

impl PrimitiveType {
    /// Parse the contract spelling of a primitive type.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "float64" => Self::Float64,
            "float32" => Self::Float32,
            "int32" => Self::Int32,
            "int64" => Self::Int64,
            "uint32" => Self::Uint32,
            "uint64" => Self::Uint64,
            "bool" => Self::Bool,
            "string" => Self::Str,
            _ => return None,
        })
    }

    /// The canonical contract spelling.
    pub const fn as_contract_str(self) -> &'static str {
        match self {
            Self::Float64 => "float64",
            Self::Float32 => "float32",
            Self::Int32 => "int32",
            Self::Int64 => "int64",
            Self::Uint32 => "uint32",
            Self::Uint64 => "uint64",
            Self::Bool => "bool",
            Self::Str => "string",
        }
    }

    /// The generated Rust type spelling.
    pub const fn rust_type(self) -> &'static str {
        match self {
            Self::Float64 => "f64",
            Self::Float32 => "f32",
            Self::Int32 => "i32",
            Self::Int64 => "i64",
            Self::Uint32 => "u32",
            Self::Uint64 => "u64",
            Self::Bool => "bool",
            Self::Str => "String",
        }
    }

    /// Whether the generated Rust type implements `Copy`.
    pub const fn is_copy(self) -> bool {
        !matches!(self, Self::Str)
    }

    /// Every supported spelling, for building diagnostic messages.
    pub const ALL: &'static [&'static str] = &[
        "float64", "float32", "int32", "int64", "uint32", "uint64", "bool", "string",
    ];
}

/// The clock-domain vocabulary accepted in contracts.
///
/// This mirrors the domains implemented by `neuradix-time`. The two crates
/// deliberately do not depend on one another (contracts is the dependency
/// root); the shared vocabulary is documented in
/// `docs/rfcs/RFC-0003-Time-Clocks-and-Deterministic-Replay.md` and enforced by
/// a test in `neuradix-testkit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockDomainRef {
    /// Monotonic execution time.
    Monotonic,
    /// UTC / wall-clock time.
    Utc,
    /// Sensor hardware time.
    Sensor,
    /// Simulation time.
    Simulation,
    /// Replay time.
    Replay,
}

impl ClockDomainRef {
    /// Parse the contract spelling of a clock domain.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "monotonic" => Self::Monotonic,
            "utc" => Self::Utc,
            "sensor" => Self::Sensor,
            "simulation" => Self::Simulation,
            "replay" => Self::Replay,
            _ => return None,
        })
    }

    /// The canonical contract spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monotonic => "monotonic",
            Self::Utc => "utc",
            Self::Sensor => "sensor",
            Self::Simulation => "simulation",
            Self::Replay => "replay",
        }
    }

    /// Every supported spelling, for building diagnostic messages.
    pub const ALL: &'static [&'static str] =
        &["monotonic", "utc", "sensor", "simulation", "replay"];
}

/// Semantic metadata attached to a stream contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Semantics {
    /// Coordinate frame identifier, preserved verbatim (e.g. `vehicle/base`).
    pub frame: String,
    /// The clock domain of the authoritative timestamp.
    pub clock_domain: ClockDomainRef,
    /// Which timestamp is authoritative (e.g. `measurement`).
    pub authoritative_timestamp: String,
    /// Maximum age before a sample is considered stale.
    pub maximum_age: Duration,
}

/// Delivery / queueing policy for a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery {
    /// Bounded queue capacity (must be greater than zero).
    pub capacity: NonZeroU32,
    /// Behaviour when the queue is full.
    pub overflow: OverflowPolicy,
}

/// Overflow behaviour for a bounded stream.
///
/// This type lives in `neuradix-contracts` because overflow is a contract-level
/// delivery property; the transport layer reuses it so the authored policy and
/// the runtime behaviour cannot drift apart. Precise per-policy semantics are
/// specified in `docs/rfcs/RFC-0004-Transport-Neutral-Data-Plane.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Refuse the incoming item when full (counted as a rejection).
    Reject,
    /// Evict the oldest queued item, then enqueue the new one.
    DropOldest,
    /// Drop the incoming item when full (counted as a drop).
    DropNewest,
    /// Retain only the single most recent item (latest-value semantics).
    KeepLatest,
}

impl OverflowPolicy {
    /// Parse the contract spelling of an overflow policy.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "reject" => Self::Reject,
            "drop-oldest" => Self::DropOldest,
            "drop-newest" => Self::DropNewest,
            "keep-latest" => Self::KeepLatest,
            _ => return None,
        })
    }

    /// The canonical contract spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reject => "reject",
            Self::DropOldest => "drop-oldest",
            Self::DropNewest => "drop-newest",
            Self::KeepLatest => "keep-latest",
        }
    }

    /// Every supported spelling, for building diagnostic messages.
    pub const ALL: &'static [&'static str] =
        &["reject", "drop-oldest", "drop-newest", "keep-latest"];
}

/// A non-negative duration stored as whole nanoseconds.
///
/// Contracts carry a self-contained duration type (rather than depending on
/// `neuradix-time`) so that `neuradix-contracts` remains the dependency root.
/// Durations are normalised to nanoseconds so that `100ms` and `0.1s` produce
/// identical schema identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration {
    nanos: u128,
}

impl Duration {
    /// Construct from whole nanoseconds.
    pub const fn from_nanos(nanos: u128) -> Self {
        Self { nanos }
    }

    /// The duration in whole nanoseconds.
    pub const fn as_nanos(self) -> u128 {
        self.nanos
    }

    /// Parse a duration literal such as `100ms`, `0.1s`, `500us`, `2m`.
    ///
    /// Accepted unit suffixes: `ns`, `us`, `ms`, `s`, `m`, `h`. A bare number
    /// is rejected: an explicit unit is required so intent is unambiguous.
    pub fn parse(input: &str) -> Option<Self> {
        let s = input.trim();
        if s.is_empty() {
            return None;
        }
        // Longest suffixes first so `ms`/`us`/`ns` win over `s`.
        const UNITS: &[(&str, u128)] = &[
            ("ns", 1),
            ("us", 1_000),
            ("ms", 1_000_000),
            ("s", 1_000_000_000),
            ("m", 60_000_000_000),
            ("h", 3_600_000_000_000),
        ];
        let (number, scale) = UNITS
            .iter()
            .find_map(|(suffix, scale)| s.strip_suffix(suffix).map(|n| (n.trim(), *scale)))?;
        if number.is_empty() {
            return None;
        }
        // Support integer and simple decimal magnitudes without floating point,
        // to keep normalisation exact and deterministic.
        let nanos = parse_decimal_scaled(number, scale)?;
        Some(Self { nanos })
    }
}

/// Parse a non-negative decimal `number` and multiply by `scale` nanoseconds,
/// exactly (no floating point), returning whole nanoseconds.
fn parse_decimal_scaled(number: &str, scale: u128) -> Option<u128> {
    if number.starts_with('-') {
        return None;
    }
    let (int_part, frac_part) = match number.split_once('.') {
        Some((i, f)) => (i, f),
        None => (number, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.is_empty() && !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac_part.is_empty() && !frac_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let int_value: u128 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().ok()?
    };
    let mut total = int_value.checked_mul(scale)?;
    if !frac_part.is_empty() {
        // Add the fractional contribution: (frac / 10^len) * scale, exactly.
        let frac_value: u128 = frac_part.parse().ok()?;
        let divisor: u128 = 10u128.checked_pow(frac_part.len() as u32)?;
        let scaled = frac_value.checked_mul(scale)?;
        // Require exact division so we never silently lose sub-nanosecond parts.
        if scaled % divisor != 0 {
            return None;
        }
        total = total.checked_add(scaled / divisor)?;
    }
    Some(total)
}
