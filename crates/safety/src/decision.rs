//! Command requests and the auditable safety decisions they produce.

use neuradix_time::Timestamp;

use crate::authority::{Capability, Identity};

/// A request to command a capability to a value at a time.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRequest {
    /// The command source.
    pub holder: Identity,
    /// The controlled capability.
    pub capability: Capability,
    /// The requested value.
    pub value: f64,
    /// When the command was issued (domain-tagged).
    pub at: Timestamp,
}

impl CommandRequest {
    /// Construct a command request.
    pub fn new(holder: Identity, capability: Capability, value: f64, at: Timestamp) -> Self {
        Self {
            holder,
            capability,
            value,
            at,
        }
    }
}

/// The reason a command was rejected (and forced to a safe output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// Authority was denied.
    Authority(crate::authority::AuthorityDenial),
}

impl std::fmt::Display for RejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RejectReason::Authority(d) => write!(f, "authority denied: {d}"),
        }
    }
}

/// The outcome of a safety evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Authorized and within all constraints unchanged.
    Accepted,
    /// Authorized but modified (clamped) by one or more constraints.
    Modified,
    /// Rejected; a fail-safe value was applied instead.
    Rejected(RejectReason),
}

impl Outcome {
    /// A short, stable label.
    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Accepted => "accepted",
            Outcome::Modified => "modified",
            Outcome::Rejected(_) => "rejected",
        }
    }
}

/// The immutable, auditable result of evaluating a command through the safety
/// path (§16.7, §25.3). It carries the originating request, the outcome, the
/// value actually applied, and the identifiers of the rules that acted — enough
/// to explain the decision and to replay it.
#[derive(Debug, Clone, PartialEq)]
pub struct SafetyDecision {
    /// The originating command request.
    pub request: CommandRequest,
    /// The outcome classification.
    pub outcome: Outcome,
    /// The value actually applied (a fail-safe value when rejected).
    pub applied: f64,
    /// Identifiers of the constraint rules that modified the value, in order.
    pub acted_rules: Vec<&'static str>,
    /// The decision time.
    pub at: Timestamp,
}

impl SafetyDecision {
    /// Whether the command was rejected.
    pub fn is_rejected(&self) -> bool {
        matches!(self.outcome, Outcome::Rejected(_))
    }
}
