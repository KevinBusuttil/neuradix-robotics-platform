//! Causal command lineage (§25.3–§25.4).
//!
//! A [`CommandLineage`] is a self-describing record of one actuator command and
//! the chain that produced it: the originating sensor input, the requested
//! command value, the authority + constraint outcome, and the value finally
//! applied. It is serialized as JSON so a recording can be explained later
//! without the reader needing any compiled Rust type — this is the data behind
//! `neuradix explain command`.
//!
//! The lineage is a deliberately owned, primitive-typed data-transfer object
//! (not the in-memory [`SafetyDecision`], which uses `&'static str` rule ids)
//! so it round-trips through serialization cleanly and forms a stable format.

use serde::{Deserialize, Serialize};

use crate::decision::{Outcome, SafetyDecision};

/// The well-known recording channel name for command lineage records.
pub const LINEAGE_CHANNEL: &str = "safety/command-lineage";

/// The originating sensor input that drove a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineageOrigin {
    /// The source contract/stream, e.g. `navigation/vehicle-depth`.
    pub source: String,
    /// The physical quantity, e.g. `depth`.
    pub quantity: String,
    /// The unit, e.g. `m`.
    pub unit: String,
    /// The value.
    pub value: f64,
}

impl LineageOrigin {
    /// Construct a lineage origin.
    pub fn new(
        source: impl Into<String>,
        quantity: impl Into<String>,
        unit: impl Into<String>,
        value: f64,
    ) -> Self {
        Self {
            source: source.into(),
            quantity: quantity.into(),
            unit: unit.into(),
            value,
        }
    }
}

/// A self-describing, serializable record of one command's causal chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandLineage {
    /// A per-mission trace/sequence identifier for the command.
    pub trace: u64,
    /// The decision time, in nanoseconds since the domain epoch.
    pub at_nanos: i128,
    /// The clock domain of the decision time.
    pub clock_domain: String,
    /// The originating sensor input.
    pub origin: LineageOrigin,
    /// The command source identity.
    pub holder: String,
    /// The controlled capability.
    pub capability: String,
    /// The value the source requested.
    pub requested: f64,
    /// The outcome (`accepted` / `modified` / `rejected`).
    pub outcome: String,
    /// The value actually applied (fail-safe when rejected).
    pub applied: f64,
    /// Identifiers of the constraint rules that modified the value.
    pub acted_rules: Vec<String>,
    /// The rejection reason, if the command was rejected.
    pub reject_reason: Option<String>,
}

impl CommandLineage {
    /// Assemble a lineage record from a sensor origin and a safety decision.
    pub fn from_decision(trace: u64, origin: LineageOrigin, decision: &SafetyDecision) -> Self {
        let (outcome, reject_reason) = match &decision.outcome {
            Outcome::Accepted => ("accepted".to_owned(), None),
            Outcome::Modified => ("modified".to_owned(), None),
            Outcome::Rejected(reason) => ("rejected".to_owned(), Some(reason.to_string())),
        };
        Self {
            trace,
            at_nanos: decision.at.as_nanos(),
            clock_domain: decision.at.domain().as_str().to_owned(),
            origin,
            holder: decision.request.holder.as_str().to_owned(),
            capability: decision.request.capability.as_str().to_owned(),
            requested: decision.request.value,
            outcome,
            applied: decision.applied,
            acted_rules: decision
                .acted_rules
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            reject_reason,
        }
    }

    /// Serialize to JSON payload bytes for recording.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("CommandLineage is always serializable")
    }

    /// Deserialize from JSON payload bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthorityLease, Capability, CommandRequest, Constraint, Identity, LeaseTable, SafetyGate,
    };
    use neuradix_time::{ClockDomain, Timestamp};

    #[test]
    fn lineage_captures_the_chain_and_round_trips() {
        let holder = Identity::new("depth-controller");
        let cap = Capability::new("propulsion/vertical-thrust");
        let mut leases = LeaseTable::new();
        leases.grant(AuthorityLease {
            holder: holder.clone(),
            capability: cap.clone(),
            priority: 1,
            issued: Timestamp::new(ClockDomain::Simulation, 0),
            expires: Timestamp::new(ClockDomain::Simulation, 1_000_000_000),
            envelope: None,
        });
        let mut gate = SafetyGate::new(
            leases,
            vec![Constraint::range("range", -2.0, 2.0).unwrap()],
            0.0,
        );

        let decision = gate.evaluate(CommandRequest::new(
            holder,
            cap,
            9.0,
            Timestamp::new(ClockDomain::Simulation, 500),
        ));
        let origin = LineageOrigin::new("navigation/vehicle-depth", "depth", "m", 3.0);
        let lineage = CommandLineage::from_decision(7, origin, &decision);

        assert_eq!(lineage.trace, 7);
        assert_eq!(lineage.outcome, "modified");
        assert_eq!(lineage.requested, 9.0);
        assert_eq!(lineage.applied, 2.0);
        assert_eq!(lineage.acted_rules, vec!["range".to_owned()]);
        assert_eq!(lineage.at_nanos, 500);

        let bytes = lineage.to_json_bytes();
        let back = CommandLineage::from_json_bytes(&bytes).unwrap();
        assert_eq!(lineage, back, "lineage must round-trip through JSON");
    }
}
