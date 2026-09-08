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
    #[serde(with = "requested_value")]
    pub requested: f64,
    /// Source validity metadata; absent in recordings produced before A04.2.
    #[serde(default)]
    pub command_metadata: Option<CommandMetadata>,
    /// The outcome (`accepted` / `modified` / `rejected`).
    pub outcome: String,
    /// The value actually applied (fail-safe when rejected).
    pub applied: f64,
    /// Identifiers of the constraint rules that modified the value.
    pub acted_rules: Vec<String>,
    /// The rejection reason, if the command was rejected.
    pub reject_reason: Option<String>,
}

/// Recorded source validity metadata, separate from runtime decision time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandMetadata {
    /// Original source timestamp in nanoseconds.
    pub source_at_nanos: i128,
    /// Source clock domain.
    pub source_clock_domain: String,
    /// Exclusive source deadline.
    pub deadline_nanos: i128,
    /// Deadline clock domain.
    pub deadline_clock_domain: String,
    /// Shared-timeline identity (not authentication).
    pub timeline: u64,
    /// Trusted generation echoed by the command, encoded as decimal text.
    pub generation: String,
    /// Command sequence number.
    pub sequence: u64,
}

impl CommandLineage {
    /// Assemble command lineage, or None for an idle evaluation decision.
    /// Idle decisions still carry an auditable runtime timestamp/reason in SafetyDecision.
    pub fn from_decision(
        trace: u64,
        origin: LineageOrigin,
        decision: &SafetyDecision,
    ) -> Option<Self> {
        let request = decision.request.as_ref()?;
        let (outcome, reject_reason) = match &decision.outcome {
            Outcome::Accepted => ("accepted".to_owned(), None),
            Outcome::Modified => ("modified".to_owned(), None),
            Outcome::Rejected(reason) => ("rejected".to_owned(), Some(reason.to_string())),
        };
        Some(Self {
            trace,
            at_nanos: decision.at.as_nanos(),
            clock_domain: decision.at.domain().as_str().to_owned(),
            origin,
            holder: request.holder.as_str().to_owned(),
            capability: request.capability.as_str().to_owned(),
            requested: request.value,
            command_metadata: Some(CommandMetadata {
                source_at_nanos: request.meta.source_at.as_nanos(),
                source_clock_domain: request.meta.source_at.domain().as_str().to_owned(),
                deadline_nanos: request.meta.deadline.as_nanos(),
                deadline_clock_domain: request.meta.deadline.domain().as_str().to_owned(),
                timeline: request.meta.timeline,
                generation: request.meta.generation.get().to_string(),
                sequence: request.meta.sequence,
            }),
            outcome,
            applied: decision.applied,
            acted_rules: decision
                .acted_rules
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            reject_reason,
        })
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

// JSON numbers cannot represent NaN/infinity. Explicit markers keep rejected
// command evidence readable and round-trippable instead of serializing to null.
mod requested_value {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() {
            serializer.serialize_f64(*value)
        } else {
            serializer.serialize_str(if value.is_nan() {
                "NaN"
            } else if value.is_sign_positive() {
                "Infinity"
            } else {
                "-Infinity"
            })
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Number(f64),
            Marker(String),
        }
        match Value::deserialize(deserializer)? {
            Value::Number(value) => Ok(value),
            Value::Marker(value) => match value.as_str() {
                "NaN" => Ok(f64::NAN),
                "Infinity" => Ok(f64::INFINITY),
                "-Infinity" => Ok(f64::NEG_INFINITY),
                _ => Err(serde::de::Error::custom(
                    "unknown non-finite command marker",
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capability, CommandMeta, CommandRequest, Generation, Identity, RejectReason};
    use neuradix_time::{ClockDomain, Timestamp};
    fn decision(value: f64) -> SafetyDecision {
        let source = Timestamp::new(ClockDomain::Simulation, 100);
        SafetyDecision {
            request: Some(CommandRequest::new(
                Identity::new("controller"),
                Capability::new("thrust"),
                value,
                CommandMeta {
                    generation: Generation::new(7).unwrap(),
                    sequence: 3,
                    source_at: source,
                    deadline: Timestamp::new(ClockDomain::Simulation, 500),
                    timeline: 9,
                },
            )),
            outcome: Outcome::Rejected(RejectReason::NonFiniteCommand),
            applied: 0.0,
            acted_rules: vec![],
            at: Timestamp::new(ClockDomain::Simulation, 200),
        }
    }
    #[test]
    fn lineage_keeps_source_and_evaluation_times_separate() {
        let lineage = CommandLineage::from_decision(
            7,
            LineageOrigin::new("depth", "depth", "m", 3.0),
            &decision(9.0),
        )
        .unwrap();
        assert_eq!(lineage.at_nanos, 200);
        let meta = lineage.command_metadata.as_ref().unwrap();
        assert_eq!(meta.source_at_nanos, 100);
        assert_eq!(meta.deadline_nanos, 500);
        assert_eq!(meta.generation, "7");
        assert_eq!(meta.sequence, 3);
        assert_eq!(
            CommandLineage::from_json_bytes(&lineage.to_json_bytes()).unwrap(),
            lineage
        );
        let mut old: serde_json::Value = serde_json::from_slice(&lineage.to_json_bytes()).unwrap();
        old.as_object_mut().unwrap().remove("commandMetadata");
        assert!(
            CommandLineage::from_json_bytes(&serde_json::to_vec(&old).unwrap())
                .unwrap()
                .command_metadata
                .is_none()
        );
    }
    #[test]
    fn non_finite_rejections_survive_recording_round_trip() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let lineage = CommandLineage::from_decision(
                0,
                LineageOrigin::new("depth", "depth", "m", 3.0),
                &decision(value),
            )
            .unwrap();
            let back = CommandLineage::from_json_bytes(&lineage.to_json_bytes()).unwrap();
            assert_eq!(back.applied, 0.0);
            assert_eq!(back.reject_reason.as_deref(), Some("NonFiniteCommand"));
            if value.is_nan() {
                assert!(back.requested.is_nan());
            } else {
                assert_eq!(back.requested, value);
            }
        }
    }
    #[test]
    fn idle_decision_has_no_invented_command_lineage() {
        let mut d = decision(0.5);
        d.request = None;
        assert!(
            CommandLineage::from_decision(0, LineageOrigin::new("depth", "depth", "m", 3.0), &d)
                .is_none()
        );
    }
}
