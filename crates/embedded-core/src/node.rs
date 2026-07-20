//! The executor-neutral component trait and the reference propulsion node.
//!
//! [`EmbeddedComponent`] carries no executor: a host static loop, Embassy or
//! RTIC each call [`EmbeddedComponent::tick`] at the control period. This keeps
//! the component logic identical across host simulation and real firmware.

use neuradix_time::Timestamp;

use crate::gate::{CommandGate, GateDecision, Outcome};
use crate::health::HealthState;
use crate::identity::NodeId;

/// A statically-allocated, executor-neutral MCU component.
pub trait EmbeddedComponent {
    /// The node's stable identity.
    fn id(&self) -> NodeId;

    /// The node's current health.
    fn health(&self) -> HealthState;

    /// Advance one control tick at `now`, given the latest command `request`
    /// (`None` if no fresh command arrived this tick), and return the value
    /// applied to the actuator.
    fn tick(&mut self, now: Timestamp, request: Option<f32>) -> f32;
}

/// The reference AUV propulsion node (§ Embedded Profile "Reference
/// demonstration").
///
/// It validates the authority lease, keeps the link alive with a watchdog,
/// enforces the thrust envelope (range + slew), applies the output, reports
/// health, and enters its **local safe state** on lease expiry or link loss —
/// all through the [`CommandGate`], with no dependency on the host.
#[derive(Debug, Clone, Copy)]
pub struct PropulsionNode {
    id: NodeId,
    gate: CommandGate,
    last: Option<GateDecision>,
}

impl PropulsionNode {
    /// Build a propulsion node from its identity and command gate.
    pub fn new(id: NodeId, gate: CommandGate) -> Self {
        Self {
            id,
            gate,
            last: None,
        }
    }

    /// The most recent gate decision, if the node has ticked at least once.
    pub fn last_decision(&self) -> Option<GateDecision> {
        self.last
    }

    /// Whether the node is currently applying its local safe output.
    pub fn in_safe_state(&self) -> bool {
        matches!(self.last, Some(d) if matches!(d.outcome, Outcome::SafeState(_)))
    }
}

impl EmbeddedComponent for PropulsionNode {
    fn id(&self) -> NodeId {
        self.id
    }

    fn health(&self) -> HealthState {
        match self.last {
            None => HealthState::Unknown,
            Some(decision) => match decision.outcome {
                // A node holding its safe output is operating with reduced
                // capability, not failed — it is doing exactly the right thing.
                Outcome::SafeState(_) => HealthState::Degraded,
                Outcome::Accepted | Outcome::Modified => HealthState::Healthy,
            },
        }
    }

    fn tick(&mut self, now: Timestamp, request: Option<f32>) -> f32 {
        let decision = self.gate.evaluate(request, now);
        self.last = Some(decision);
        decision.applied
    }
}
