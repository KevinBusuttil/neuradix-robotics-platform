//! Command requests and the auditable safety decisions they produce.

use neuradix_command_core::CommandMeta;
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
    /// Original source time, deadline, sequence, generation and timeline identity.
    pub meta: CommandMeta,
}

impl CommandRequest {
    /// Construct a command request.
    pub fn new(holder: Identity, capability: Capability, value: f64, meta: CommandMeta) -> Self {
        Self {
            holder,
            capability,
            value,
            meta,
        }
    }
}

/// Shared host/embedded rejection vocabulary.
pub use neuradix_command_core::CommandRejection as RejectReason;

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
    /// Incoming command, or None on an idle evaluation tick.
    pub request: Option<CommandRequest>,
    /// The outcome classification.
    pub outcome: Outcome,
    /// The value actually applied (a fail-safe value when rejected).
    pub applied: f64,
    /// Identifiers of the constraint rules that modified the value, in order.
    pub acted_rules: Vec<&'static str>,
    /// Runtime-owned evaluation time, including the supplied time on rejection.
    pub at: Timestamp,
}

impl SafetyDecision {
    /// Whether the command was rejected.
    pub fn is_rejected(&self) -> bool {
        matches!(self.outcome, Outcome::Rejected(_))
    }
}
