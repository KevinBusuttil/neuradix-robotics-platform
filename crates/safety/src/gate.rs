//! Bounded command evaluation and local fallback for a host actuator path.

use neuradix_command_core::{EvaluationClock, Generation};
use neuradix_runtime::{ComponentError, Processor, TickContext};
use neuradix_time::Timestamp;

use crate::authority::LeaseTable;
use crate::constraint::Constraint;
use crate::decision::{CommandRequest, Outcome, RejectReason, SafetyDecision};
use crate::error::SafetyError;

/// Host command gate. Invoke on a bounded periodic schedule even without input.
///
/// Runtime clock faults latch until reconstruction with a non-reused generation.
/// Ordinary rejection applies safe output until a valid new command arrives.
#[derive(Debug, Clone)]
pub struct SafetyGate {
    leases: LeaseTable,
    constraints: Vec<Constraint>,
    safe_value: f64,
    last_applied: Option<f64>,
    clock: EvaluationClock,
    active: Option<(usize, Generation)>,
    fallback_reason: Option<RejectReason>,
}
impl SafetyGate {
    /// Construct only with a finite safe value satisfying every hard output range.
    pub fn new(leases: LeaseTable, constraints: Vec<Constraint>, safe_value: f64) -> Result<Self, SafetyError> {
        if !safe_value.is_finite() || !constraints.iter().all(|c| c.permits_output(safe_value)) {
            return Err(SafetyError::InvalidSafeOutput);
        }
        Ok(Self { leases, constraints, safe_value, last_applied: None, clock: EvaluationClock::default(), active: None, fallback_reason: None })
    }
    /// Trusted control-plane access. Never dispatch these operations from payloads.
    pub fn leases_mut(&mut self) -> &mut LeaseTable { &mut self.leases }
    /// Read-only authority and accepted-command diagnostics.
    pub fn leases(&self) -> &LeaseTable { &self.leases }
    /// Most recently applied output.
    pub fn last_applied(&self) -> Option<f64> { self.last_applied }
    /// Evaluate new input or an idle tick at runtime-owned `now`. Both expire held
    /// authority/freshness/deadlines; only a fully valid new command feeds liveness.
    pub fn evaluate(&mut self, request: Option<CommandRequest>, now: Timestamp) -> SafetyDecision {
        let elapsed = match self.clock.observe(now) {
            Ok(dt) => dt,
            Err(reason) => return self.reject(request, now, reason, Vec::new()),
        };
        let Some(input) = request.as_ref() else {
            let validity = match self.active {
                Some((index, generation)) => self.leases.check_held(index, generation, now),
                None => Err(RejectReason::NoCommand),
            };
            if let Err(reason) = validity { return self.reject(None, now, reason, Vec::new()); }
            if let Some(reason) = self.fallback_reason { return self.reject(None, now, reason, Vec::new()); }
            return SafetyDecision { request: None, outcome: Outcome::Accepted, applied: self.last_applied.unwrap_or(self.safe_value), acted_rules: Vec::new(), at: now };
        };
        let index = match self.leases.validate(&input.holder, &input.capability, input.meta, now, input.value) {
            Ok(index) => index,
            Err(reason) => return self.reject(request, now, reason, Vec::new()),
        };
        let previous = self.last_applied.zip(elapsed);
        let mut value = input.value;
        let mut acted_rules = Vec::new();
        for constraint in &self.constraints {
            let Some(constrained) = constraint.apply(value, previous) else {
                return self.reject(request, now, RejectReason::InvalidOutput, acted_rules);
            };
            if constrained != value { acted_rules.push(constraint.id()); }
            value = constrained;
        }
        if !value.is_finite() || !self.constraints.iter().all(|c| c.permits_output(value)) {
            return self.reject(request, now, RejectReason::InvalidOutput, acted_rules);
        }
        if let Err(reason) = self.leases.accept(index, input.meta, now) {
            return self.reject(request, now, reason, acted_rules);
        }
        self.active = Some((index, input.meta.generation));
        self.last_applied = Some(value);
        self.fallback_reason = None;
        let outcome = if acted_rules.is_empty() { Outcome::Accepted } else { Outcome::Modified };
        SafetyDecision { request, outcome, applied: value, acted_rules, at: now }
    }
    fn reject(&mut self, request: Option<CommandRequest>, now: Timestamp, reason: RejectReason, acted_rules: Vec<&'static str>) -> SafetyDecision {
        self.last_applied = Some(self.safe_value);
        self.fallback_reason = Some(reason);
        SafetyDecision { request, outcome: Outcome::Rejected(reason), applied: self.safe_value, acted_rules, at: now }
    }
}
impl Processor for SafetyGate {
    type Input = Option<CommandRequest>;
    type Output = SafetyDecision;
    fn process(&mut self, ctx: &TickContext, input: Self::Input) -> Result<Vec<SafetyDecision>, ComponentError> {
        Ok(vec![self.evaluate(input, ctx.now)])
    }
}
