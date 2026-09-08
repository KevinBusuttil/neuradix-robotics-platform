//! The safety gate: the authority + constraint path every command traverses.

use neuradix_runtime::{ComponentError, Processor, TickContext};
use neuradix_time::Timestamp;

use crate::authority::LeaseTable;
use crate::constraint::Constraint;
use crate::decision::{CommandRequest, Outcome, RejectReason, SafetyDecision};
use crate::error::SafetyError;

/// A deterministic authority and constraint gate using runtime-owned time.
///
/// Source timestamps are diagnostic only. Evaluation clock changes, regression
/// and elapsed-time overflow latch a fault until a new gate is constructed.
/// Rejections apply the validated safe output immediately, bypassing slew.
#[derive(Debug, Clone)]
pub struct SafetyGate {
    leases: LeaseTable,
    constraints: Vec<Constraint>,
    safe_value: f64,
    last_applied: Option<(f64, Timestamp)>,
    time_fault: Option<RejectReason>,
}

impl SafetyGate {
    /// Build a gate. The safe value must be finite and satisfy every hard range;
    /// incompatible ranges therefore cannot create a usable gate.
    pub fn new(
        leases: LeaseTable,
        constraints: Vec<Constraint>,
        safe_value: f64,
    ) -> Result<Self, SafetyError> {
        if !safe_value.is_finite() || !constraints.iter().all(|c| c.permits_output(safe_value)) {
            return Err(SafetyError::InvalidSafeOutput);
        }
        Ok(Self {
            leases,
            constraints,
            safe_value,
            last_applied: None,
            time_fault: None,
        })
    }

    /// Mutable access to the lease table (e.g. to grant or revoke authority).
    pub fn leases_mut(&mut self) -> &mut LeaseTable {
        &mut self.leases
    }

    /// The most recently applied value, if any.
    pub fn last_applied(&self) -> Option<f64> {
        self.last_applied.map(|(v, _)| v)
    }

    /// Evaluate at runtime-owned `now`, independently of `request.at`.
    ///
    /// Equal evaluation times are allowed (zero elapsed time). Source time may
    /// use another domain: no age/skew policy is implemented in this increment.
    /// Never populate `now` from untrusted command metadata.
    pub fn evaluate(&mut self, request: CommandRequest, now: Timestamp) -> SafetyDecision {
        let previous = match self.last_applied {
            Some((value, at)) => {
                let fault = if now.domain() != at.domain() {
                    Some(RejectReason::EvaluationClockMismatch)
                } else if now.as_nanos() < at.as_nanos() {
                    Some(RejectReason::EvaluationTimeRegression)
                } else {
                    None
                };
                if self.time_fault.is_none() {
                    self.time_fault = fault;
                }
                match now.duration_since(at) {
                    Ok(dt) => Some((value, dt)),
                    Err(_) => {
                        if self.time_fault.is_none() {
                            self.time_fault = Some(RejectReason::EvaluationTimeOverflow);
                        }
                        None
                    }
                }
            }
            None => None,
        };
        if let Some(reason) = self.time_fault {
            return self.reject(request, now, reason, Vec::new());
        }
        if !request.value.is_finite() {
            return self.reject(request, now, RejectReason::NonFiniteCommand, Vec::new());
        }
        if let Err(denial) = self.leases.authorize(
            &request.holder,
            &request.capability,
            now,
            request.value,
        ) {
            return self.reject(request, now, RejectReason::Authority(denial), Vec::new());
        }

        let mut value = request.value;
        let mut acted_rules = Vec::new();
        for constraint in &self.constraints {
            let Some(constrained) = constraint.apply(value, previous) else {
                return self.reject(request, now, RejectReason::InvalidOutput, acted_rules);
            };
            if constrained != value {
                acted_rules.push(constraint.id());
            }
            value = constrained;
        }
        if !value.is_finite() || !self.constraints.iter().all(|c| c.permits_output(value)) {
            return self.reject(request, now, RejectReason::InvalidOutput, acted_rules);
        }
        let outcome = if acted_rules.is_empty() {
            Outcome::Accepted
        } else {
            Outcome::Modified
        };
        self.last_applied = Some((value, now));
        SafetyDecision {
            request,
            outcome,
            applied: value,
            acted_rules,
            at: now,
        }
    }

    fn reject(
        &mut self,
        request: CommandRequest,
        now: Timestamp,
        reason: RejectReason,
        acted_rules: Vec<&'static str>,
    ) -> SafetyDecision {
        // Keep the last valid evaluation reference when the clock faults. The
        // fault remains latched even if a later timestamp appears valid again.
        let at = if self.time_fault.is_some() {
            self.last_applied.map_or(now, |(_, at)| at)
        } else {
            now
        };
        self.last_applied = Some((self.safe_value, at));
        SafetyDecision {
            request,
            outcome: Outcome::Rejected(reason),
            applied: self.safe_value,
            acted_rules,
            at: now,
        }
    }
}

impl Processor for SafetyGate {
    type Input = CommandRequest;
    type Output = SafetyDecision;

    fn process(
        &mut self,
        ctx: &TickContext,
        input: CommandRequest,
    ) -> Result<Vec<SafetyDecision>, ComponentError> {
        Ok(vec![self.evaluate(input, ctx.now)])
    }
}
