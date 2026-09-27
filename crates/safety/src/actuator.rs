//! Trusted ownership boundary for one scalar host actuator driver.
//!
//! Setup owns the adapter and issues scoped command ports. Components receive
//! only a port; payloads cannot grant permission, choose a port's mode, submit a
//! precomputed decision or access the driver. Native code/OS isolation and physical
//! behavior are outside this API guarantee. Driver implementations are trusted.

use crate::{
    AuthorityLease, Capability, CommandRequest, Constraint, Generation, Identity, LeaseTable,
    RejectReason, SafetyDecision, SafetyGate, SessionError,
};
use neuradix_time::{ClockDomain, Timestamp};

/// Maximum UTF-8 bytes in each admitted component, capability or driver name.
pub const MAX_BINDING_NAME_BYTES: usize = 128;

/// Mode assigned by trusted composition, never decoded from a command payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Local monotonic execution on a live-designated driver.
    Live,
    /// Simulation clock and simulation-designated driver.
    Simulation,
    /// Replay clock and replay-designated driver.
    Replay,
}
impl ExecutionMode {
    /// Required evaluation and lease clock domain for this selected adapter.
    pub const fn clock_domain(self) -> ClockDomain {
        match self {
            Self::Live => ClockDomain::Monotonic,
            Self::Simulation => ClockDomain::Simulation,
            Self::Replay => ClockDomain::Replay,
        }
    }
}

/// Immutable names/mode. Describes a binding but does not itself grant permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActuatorBinding {
    holder: Identity,
    capability: Capability,
    driver: String,
    mode: ExecutionMode,
}
impl ActuatorBinding {
    /// Admit nonempty ASCII names of at most 128 bytes: letters, digits, `_-/.:`.
    /// No trimming/normalization or allocations occur before all names validate.
    pub fn new(
        holder: &str,
        capability: &str,
        driver: &str,
        mode: ExecutionMode,
    ) -> Result<Self, PermissionError> {
        for name in [holder, capability, driver] {
            if name.is_empty()
                || name.len() > MAX_BINDING_NAME_BYTES
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-/.:".contains(&b))
            {
                return Err(PermissionError::InvalidName);
            }
        }
        Ok(Self {
            holder: Identity::new(holder),
            capability: Capability::new(capability),
            driver: driver.to_owned(),
            mode,
        })
    }
    /// Bound component/source name, not an authenticated principal.
    pub fn holder(&self) -> &Identity {
        &self.holder
    }
    /// Bound scalar command capability.
    pub fn capability(&self) -> &Capability {
        &self.capability
    }
    /// Selected endpoint label, verified against the trusted driver's descriptor.
    pub fn driver(&self) -> &str {
        &self.driver
    }
    /// Immutable execution mode.
    pub fn mode(&self) -> ExecutionMode {
        self.mode
    }
}

/// Private validated output configuration; exactly two constraints are retained.
#[derive(Debug, Clone)]
pub struct ActuatorConfig {
    binding: ActuatorBinding,
    constraints: [Constraint; 2],
    safe: f64,
}
impl ActuatorConfig {
    /// Require finite ordered hard bounds, finite nonnegative rate and an in-range
    /// finite safe output. Safing bypasses slew; recovery starts from safe output.
    pub fn new(
        binding: ActuatorBinding,
        min: f64,
        max: f64,
        rate: f64,
        safe: f64,
    ) -> Result<Self, PermissionError> {
        if !min.is_finite()
            || !max.is_finite()
            || min > max
            || !rate.is_finite()
            || rate < 0.0
            || !safe.is_finite()
            || safe < min
            || safe > max
        {
            return Err(PermissionError::InvalidOutputConfig);
        }
        Ok(Self {
            binding,
            constraints: [
                Constraint::range("actuator.range", min, max).expect("validated"),
                Constraint::slew_rate("actuator.slew", rate).expect("validated"),
            ],
            safe,
        })
    }
}

/// Explicit trusted grant. Labels or SafetyDecision values cannot substitute for
/// installation through ActuatorAdapter::grant. No Clone/serde grant API.
///
/// ```compile_fail
/// use neuradix_safety::actuator::DriverPermission;
/// let permission = DriverPermission {};
/// ```
#[derive(Debug)]
pub struct DriverPermission {
    binding: ActuatorBinding,
    lease: AuthorityLease,
}
impl DriverPermission {
    /// Trusted setup only: verify lease holder/capability and the mode's timeline.
    /// Live generation allocation must be durable and non-reused across restarts.
    pub fn new(binding: ActuatorBinding, lease: AuthorityLease) -> Result<Self, PermissionError> {
        if lease.holder() != binding.holder() || lease.capability() != binding.capability() {
            return Err(PermissionError::BindingMismatch);
        }
        if lease.config().policy().timeline().domain() != binding.mode.clock_domain() {
            return Err(PermissionError::ModeMismatch);
        }
        Ok(Self { binding, lease })
    }
}

/// Configuration/control-plane failures. Invalid grants leave existing state intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionError {
    /// Empty, oversized or unsupported identifier.
    InvalidName,
    /// Invalid finite bounds/rate/safe value.
    InvalidOutputConfig,
    /// Component/capability/endpoint did not match the fixed binding.
    BindingMismatch,
    /// Driver, lease or setup mode did not match.
    ModeMismatch,
    /// Duplicate/older generation or invalid renewal/evaluation time.
    Session(SessionError),
    /// Driver failure is latched; trusted reconstruction is required.
    DriverFault,
    /// Shutdown is terminal.
    Shutdown,
    /// Renewal cannot install a missing or revoked permission.
    NotGranted,
}
impl std::fmt::Display for PermissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PermissionError {}

/// Bounded driver error vocabulary; do not infer physical state from an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// Device/transport unavailable.
    Unavailable,
    /// Driver refused the output.
    Rejected,
    /// The write may have taken effect; outcome cannot be determined.
    UnknownOutcome,
}

/// Trusted driver contract. Methods must return within the deployment's admitted
/// budget. This synchronous API cannot bound an arbitrary blocking implementation.
/// Endpoint/mode must remain fixed, and the owner must not retain a bypass handle.
pub trait ActuatorDriver {
    /// Fixed endpoint descriptor, checked once before ownership is admitted.
    fn endpoint(&self) -> &str;
    /// Fixed execution mode, not a claim of physical attestation.
    fn mode(&self) -> ExecutionMode;
    /// Write the validated scalar. Ok acknowledges this call, not physical motion.
    fn write(&mut self, value: f64) -> Result<(), DriverError>;
}

/// Auditable permission disposition for this evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// Installed trusted binding; the command gate still decides command validity.
    Granted,
    /// No permission has ever been installed.
    Missing,
    /// Trusted revocation; a newer generation is required.
    Revoked,
    /// A simulation/replay/live port cannot cross its fixed mode boundary.
    ModeMismatch,
    /// Oversized source labels rejected before copying into gate diagnostics.
    OversizedInput,
    /// Driver error remains latched across grants and later traffic.
    DriverFault,
    /// Explicit shutdown prevents further driver calls.
    Shutdown,
    /// Permission installation/replacement established safe output, awaiting input.
    Initialized,
}

/// One bounded report. Informational only: never accepted as a dispatch credential.
/// `decision.applied` is gate-selected; inspect driver results for acknowledgement.
#[derive(Debug)]
pub struct DispatchReport {
    /// Fixed target/source/mode context, retained as evidence rather than authority.
    pub binding: ActuatorBinding,
    /// Current trusted generation, including a revoked watermark; never payload-selected.
    pub generation: Option<Generation>,
    /// Trusted evaluation timestamp, not the source timestamp.
    pub at: Timestamp,
    /// Why ingress was admitted or inhibited.
    pub permission: PermissionStatus,
    /// Gate evidence when a command or idle tick was evaluated normally.
    pub decision: Option<SafetyDecision>,
    /// Clock error during permission inhibition; normal gate faults are in decision.
    pub evaluation_fault: Option<RejectReason>,
    /// Validated value selected for the first driver call.
    pub output: f64,
    /// First call result; None means shutdown suppressed all I/O.
    pub write_result: Option<Result<(), DriverError>>,
    /// One immediate safe-output attempt after a failed first call. Never retries.
    pub fallback_result: Option<Result<(), DriverError>>,
    /// First driver fault retained even when a later safe write succeeds.
    pub latched_driver_fault: Option<DriverError>,
}

/// Owns one fixed driver and one lease slot. Not Clone; no driver/gate escape API.
/// Call periodically even without input, and explicitly shutdown before dropping.
/// The adapter does not call write during Drop; arbitrary driver methods/Drop
/// require external supervision and cannot be given a wall-time guarantee.
pub struct ActuatorAdapter<D: ActuatorDriver> {
    binding: ActuatorBinding,
    driver: D,
    gate: SafetyGate,
    safe: f64,
    status: PermissionStatus,
    driver_fault: Option<DriverError>,
    generation: Option<Generation>,
}
impl<D: ActuatorDriver> ActuatorAdapter<D> {
    /// Take exclusive driver ownership after validating its endpoint and mode.
    /// No I/O occurs here; setup must grant or tick to establish the safe output
    /// before enabling ingress. Reconstruction needs a durably newer generation.
    pub fn new(config: ActuatorConfig, driver: D) -> Result<Self, PermissionError> {
        if driver.endpoint() != config.binding.driver {
            return Err(PermissionError::BindingMismatch);
        }
        if driver.mode() != config.binding.mode {
            return Err(PermissionError::ModeMismatch);
        }
        let gate = SafetyGate::new(LeaseTable::new(), config.constraints.to_vec(), config.safe)
            .expect("validated config");
        Ok(Self {
            binding: config.binding,
            driver,
            gate,
            safe: config.safe,
            status: PermissionStatus::Missing,
            driver_fault: None,
            generation: None,
        })
    }
    /// Trusted installation/replacement only. Fixed binding; same/older generations
    /// reject, including after revocation. Always establish safe output immediately.
    /// Driver failure is in the returned report and latches, even if fallback works.
    pub fn grant(
        &mut self,
        permission: DriverPermission,
        now: Timestamp,
    ) -> Result<DispatchReport, PermissionError> {
        self.control_ready()?;
        if permission.binding != self.binding {
            return Err(PermissionError::BindingMismatch);
        }
        if now.domain() != self.binding.mode.clock_domain() {
            return Err(PermissionError::ModeMismatch);
        }
        self.gate
            .check_control_time(now)
            .map_err(PermissionError::Session)?;
        let generation = permission.lease.config().generation();
        self.gate
            .leases_mut()
            .grant(permission.lease)
            .map_err(PermissionError::Session)?;
        self.generation = Some(generation);
        self.status = PermissionStatus::Granted;
        Ok(self.inhibit(now, PermissionStatus::Initialized))
    }
    /// Trusted renewal only; keeps replay, slew and accepted-command state intact.
    pub fn renew(&mut self, expires: Timestamp, now: Timestamp) -> Result<(), PermissionError> {
        self.control_ready()?;
        if self.status != PermissionStatus::Granted {
            return Err(PermissionError::NotGranted);
        }
        self.gate
            .renew_lease(&self.binding.holder, &self.binding.capability, expires, now)
            .map_err(PermissionError::Session)
    }
    /// Trusted revocation immediately attempts safe output; retains generation.
    pub fn revoke(&mut self, now: Timestamp) -> DispatchReport {
        if self.status == PermissionStatus::Shutdown {
            return self.inhibit(now, PermissionStatus::Shutdown);
        }
        self.gate
            .leases_mut()
            .revoke(&self.binding.holder, &self.binding.capability);
        self.status = PermissionStatus::Revoked;
        self.inhibit(now, PermissionStatus::Revoked)
    }
    /// Terminal explicit shutdown. At most two calls on the first invocation;
    /// repeats and subsequent port calls perform no I/O. Drop performs none.
    pub fn shutdown(&mut self, now: Timestamp) -> DispatchReport {
        if self.status == PermissionStatus::Shutdown {
            return self.inhibit(now, PermissionStatus::Shutdown);
        }
        let mut report = self.revoke(now);
        report.permission = PermissionStatus::Shutdown;
        self.status = PermissionStatus::Shutdown;
        report
    }
    /// Trusted composition creates a single-use port with immutable mode and time. Hand
    /// components only this port, never the adapter or permission control plane.
    pub fn port(&mut self, mode: ExecutionMode, now: Timestamp) -> ActuatorPort<'_, D> {
        ActuatorPort {
            adapter: self,
            mode,
            now,
        }
    }
    /// Reserved lease slots, including revoked state; always zero or one.
    pub fn binding_count(&self) -> usize {
        self.gate.leases().binding_count()
    }
    /// Last command accepted by the gate, not last successful physical actuation.
    pub fn last_accepted_at(&self) -> Option<Timestamp> {
        self.gate
            .leases()
            .last_accepted_at(&self.binding.holder, &self.binding.capability)
    }
    /// First driver failure, latched even if a safe write was acknowledged.
    pub fn driver_fault(&self) -> Option<DriverError> {
        self.driver_fault
    }
    /// Trusted binding is immutable throughout this adapter's lifetime.
    pub fn binding(&self) -> &ActuatorBinding {
        &self.binding
    }
    fn control_ready(&self) -> Result<(), PermissionError> {
        if self.status == PermissionStatus::Shutdown {
            return Err(PermissionError::Shutdown);
        }
        if self.driver_fault.is_some() {
            return Err(PermissionError::DriverFault);
        }
        Ok(())
    }
    fn tick(
        &mut self,
        mode: ExecutionMode,
        now: Timestamp,
        input: Option<CommandRequest>,
    ) -> DispatchReport {
        let denied = if self.status == PermissionStatus::Shutdown {
            Some(PermissionStatus::Shutdown)
        } else if self.driver_fault.is_some() {
            Some(PermissionStatus::DriverFault)
        } else if mode != self.binding.mode {
            Some(PermissionStatus::ModeMismatch)
        } else if self.status != PermissionStatus::Granted {
            Some(self.status)
        } else if input.as_ref().is_some_and(|r| {
            r.holder.as_str().len() > MAX_BINDING_NAME_BYTES
                || r.capability.as_str().len() > MAX_BINDING_NAME_BYTES
        }) {
            Some(PermissionStatus::OversizedInput)
        } else {
            None
        };
        if let Some(reason) = denied {
            return self.inhibit(now, reason);
        }
        // Re-own only admitted bytes: a caller may supply a short String with an
        // arbitrarily large spare capacity. Never retain that buffer in reports.
        let input = input.map(|r| {
            CommandRequest::new(
                Identity::new(r.holder.as_str()),
                Capability::new(r.capability.as_str()),
                r.value,
                r.meta,
            )
        });
        let decision = self.gate.evaluate(input, now);
        let output = decision.applied;
        self.write_report(now, PermissionStatus::Granted, Some(decision), None, output)
    }
    fn inhibit(&mut self, now: Timestamp, reason: PermissionStatus) -> DispatchReport {
        let fault = self.gate.inhibit(now);
        self.write_report(now, reason, None, fault, self.safe)
    }
    fn write_report(
        &mut self,
        now: Timestamp,
        permission: PermissionStatus,
        decision: Option<SafetyDecision>,
        evaluation_fault: Option<RejectReason>,
        output: f64,
    ) -> DispatchReport {
        let mut report = DispatchReport {
            binding: self.binding.clone(),
            generation: self.generation,
            at: now,
            permission,
            decision,
            evaluation_fault,
            output,
            write_result: None,
            fallback_result: None,
            latched_driver_fault: self.driver_fault,
        };
        if self.status == PermissionStatus::Shutdown {
            return report;
        }
        let result = self.driver.write(output);
        report.write_result = Some(result);
        if let Err(error) = result {
            if self.driver_fault.is_none() {
                self.driver_fault = Some(error);
            }
            // Preserve the clock and sequence state, but future recovery can never
            // slew from an output whose physical write failed.
            self.gate.inhibit(now);
            report.fallback_result = Some(self.driver.write(self.safe));
        }
        report.latched_driver_fault = self.driver_fault;
        report
    }
}

/// Single-use scoped ingress with mode and time fixed by trusted setup. No grant, renewal,
/// revocation, driver access or mode mutation is exposed to the component.
///
/// ```compile_fail
/// use neuradix_safety::actuator::{ActuatorDriver, ActuatorPort, ExecutionMode};
/// fn escalate<D: ActuatorDriver>(port: &mut ActuatorPort<'_, D>) {
///     port.mode = ExecutionMode::Live;
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_safety::{actuator::{ActuatorDriver, ActuatorPort}, SafetyDecision};
/// fn forged<D: ActuatorDriver>(port: ActuatorPort<'_, D>, decision: SafetyDecision) {
///     port.tick(Some(decision));
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_safety::actuator::{ActuatorDriver, ActuatorPort};
/// use neuradix_time::Timestamp;
/// fn backdate<D: ActuatorDriver>(port: &mut ActuatorPort<'_, D>, source_at: Timestamp) {
///     port.now = source_at;
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_safety::actuator::{ActuatorDriver, ActuatorPort};
/// fn twice<D: ActuatorDriver>(port: ActuatorPort<'_, D>) {
///     port.tick(None);
///     port.tick(None);
/// }
/// ```
pub struct ActuatorPort<'a, D: ActuatorDriver> {
    adapter: &'a mut ActuatorAdapter<D>,
    mode: ExecutionMode,
    now: Timestamp,
}
impl<D: ActuatorDriver> ActuatorPort<'_, D> {
    /// Evaluate internally at trusted runtime time and attempt the validated
    /// output. There is no overload accepting a SafetyDecision or raw scalar.
    pub fn tick(self, input: Option<CommandRequest>) -> DispatchReport {
        self.adapter.tick(self.mode, self.now, input)
    }
}
