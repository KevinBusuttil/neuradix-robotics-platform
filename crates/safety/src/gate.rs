//! The safety gate: the authority + constraint path every command traverses.

use neuradix_runtime::{ComponentError, Processor, TickContext};
use neuradix_time::{Duration, Timestamp};

use crate::authority::LeaseTable;
use crate::constraint::Constraint;
use crate::decision::{CommandRequest, Outcome, RejectReason, SafetyDecision};

/// The command authority and constraint path (§16.2).
///
/// A [`CommandRequest`] is authorized against the [`LeaseTable`], then passed
/// through each [`Constraint`] in order. The gate produces a [`SafetyDecision`]:
/// accepted, modified (clamped, with the responsible rules), or rejected (with a
/// fail-safe value applied). Evaluation is a pure function of the gate's state
/// and the request, so decisions are deterministic and replayable.
#[derive(Debug, Clone)]
pub struct SafetyGate {
    leases: LeaseTable,
    constraints: Vec<Constraint>,
    safe_value: f64,
    last_applied: Option<(f64, Timestamp)>,
}

impl SafetyGate {
    /// Build a gate with a lease table, ordered constraints and a fail-safe
    /// value applied when a command is rejected.
    pub fn new(leases: LeaseTable, constraints: Vec<Constraint>, safe_value: f64) -> Self {
        Self {
            leases,
            constraints,
            safe_value,
            last_applied: None,
        }
    }

    /// Mutable access to the lease table (e.g. to grant or revoke authority).
    pub fn leases_mut(&mut self) -> &mut LeaseTable {
        &mut self.leases
    }

    /// The most recently applied value, if any.
    pub fn last_applied(&self) -> Option<f64> {
        self.last_applied.map(|(v, _)| v)
    }

    /// Evaluate a command through the authority and constraint path.
    pub fn evaluate(&mut self, request: CommandRequest) -> SafetyDecision {
        match self.leases.authorize(
            &request.holder,
            &request.capability,
            request.at,
            request.value,
        ) {
            Err(denial) => {
                // Fail-safe: apply the safe value and record the rejection.
                let applied = self.safe_value;
                self.last_applied = Some((applied, request.at));
                let at = request.at;
                SafetyDecision {
                    request,
                    outcome: Outcome::Rejected(RejectReason::Authority(denial)),
                    applied,
                    acted_rules: Vec::new(),
                    at,
                }
            }
            Ok(_lease) => {
                // The previously-applied value and elapsed time, if any. On the
                // first command there is no reference, so rate limits are no-ops
                // and hard limits (range) still govern.
                let previous = self
                    .last_applied
                    .map(|(pv, pt)| (pv, request.at.duration_since(pt).unwrap_or(Duration::ZERO)));

                let mut value = request.value;
                let mut acted_rules = Vec::new();
                for constraint in &self.constraints {
                    let constrained = constraint.apply(value, previous);
                    if constrained != value {
                        acted_rules.push(constraint.id());
                    }
                    value = constrained;
                }

                let outcome = if acted_rules.is_empty() {
                    Outcome::Accepted
                } else {
                    Outcome::Modified
                };
                self.last_applied = Some((value, request.at));
                let at = request.at;
                SafetyDecision {
                    request,
                    outcome,
                    applied: value,
                    acted_rules,
                    at,
                }
            }
        }
    }
}

impl Processor for SafetyGate {
    type Input = CommandRequest;
    type Output = SafetyDecision;

    fn process(
        &mut self,
        _ctx: &TickContext,
        input: CommandRequest,
    ) -> Result<Vec<SafetyDecision>, ComponentError> {
        Ok(vec![self.evaluate(input)])
    }
}
