//! Allocation-free trusted ownership boundary for one scalar embedded actuator.
//!
//! This is the `no_std` counterpart of the host `neuradix_safety::actuator`
//! boundary. Trusted setup owns an [`ActuatorAdapter`], which owns the driver and
//! the [`CommandGate`]. Setup installs a [`DriverPermission`] and, for each
//! scheduled call, creates a single-use [`ActuatorPort`] carrying the fixed
//! execution mode and trusted runtime time. Components receive only the port:
//! a payload cannot install permission, choose evaluation time, switch mode,
//! submit a precomputed [`GateDecision`] or reach the driver.
//!
//! Three things stay distinct in every [`DispatchReport`]:
//!
//! 1. the gate-selected output (`output`, `decision`);
//! 2. the driver call and its acknowledgement (`write_result`, `fallback_result`);
//! 3. the physical equipment state, which this API never observes or claims.
//!
//! The executor, board HAL and transport are outside this API: the driver is a
//! trait implemented by board support, and the caller decides when to tick.
//! Rust ownership does not isolate `unsafe`/native code or separately obtained
//! peripheral handles; those remain deployment obligations.
//!
//! # Example
//!
//! ```
//! use neuradix_embedded_core::{
//!     ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, CommandMeta,
//!     CommandPolicy, DriverError, DriverPermission, ExecutionMode, Generation, Limits,
//!     PermissionStatus, SessionConfig, SharedTimeline,
//! };
//! use neuradix_time::{ClockDomain, Duration, Timestamp};
//!
//! struct Pwm { last: Option<f32> }
//! impl ActuatorDriver for Pwm {
//!     fn endpoint(&self) -> u64 { 7 }
//!     fn mode(&self) -> ExecutionMode { ExecutionMode::Simulation }
//!     fn write(&mut self, value: f32) -> Result<(), DriverError> { self.last = Some(value); Ok(()) }
//! }
//! let t = |ms: i128| Timestamp::new(ClockDomain::Simulation, ms * 1_000_000);
//! let binding = ActuatorBinding::new(1, 2, 7, ExecutionMode::Simulation);
//! let mut adapter = ActuatorAdapter::new(
//!     binding, Limits::with_slew_rate(-1.0, 1.0, 10.0).unwrap(), 0.0, Pwm { last: None },
//! ).unwrap();
//! let generation = Generation::new(1).unwrap();
//! let policy = CommandPolicy::new(SharedTimeline::new(1, ClockDomain::Simulation).unwrap(),
//!     Duration::from_millis(500), Duration::ZERO, Duration::from_millis(100)).unwrap();
//! let lease = AuthorityLease::new(1, 2, SessionConfig::new(generation, t(0), t(1000), policy).unwrap());
//! // Trusted setup: installation immediately writes the safe output.
//! let init = adapter.grant(DriverPermission::new(binding, lease).unwrap(), t(0)).unwrap();
//! assert_eq!(init.permission, PermissionStatus::Initialized);
//! assert_eq!(init.write_result, Some(Ok(())));
//! // Component: only the scoped port, with mode and time fixed by setup.
//! let command = Command { holder: 1, capability: 2, value: 0.5, meta: CommandMeta {
//!     generation, sequence: 0, source_at: t(10), deadline: t(200), timeline: 1 } };
//! let report = adapter.port(ExecutionMode::Simulation, t(10)).tick(Some(command));
//! // Slew-limited from the safe reference: 10 units/s over 10 ms.
//! assert!((report.output - 0.1).abs() < 1e-6);
//! assert_eq!(report.write_result, Some(Ok(()))); // an acknowledgement, not motion
//! adapter.shutdown(t(20));
//! ```

use neuradix_time::Timestamp;

use crate::gate::{Command, CommandGate, GateDecision, Limits, SafeReason};
use crate::lease::AuthorityLease;
use neuradix_command_core::reservation::{BindingKey, ReservedGeneration};
use neuradix_command_core::{ConfigError, EvaluationClock, ExecutionMode, Generation};

/// Immutable holder/capability/endpoint/mode binding, fixed for an adapter's
/// lifetime. Numeric identifiers are deployment-owned, not credentials, and a
/// binding value alone grants nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActuatorBinding {
    holder: u64,
    capability: u64,
    endpoint: u64,
    mode: ExecutionMode,
}
impl ActuatorBinding {
    /// Describe a binding selected by trusted composition.
    pub const fn new(holder: u64, capability: u64, endpoint: u64, mode: ExecutionMode) -> Self {
        Self {
            holder,
            capability,
            endpoint,
            mode,
        }
    }
    /// Bound command source, not an authenticated principal.
    pub const fn holder(&self) -> u64 {
        self.holder
    }
    /// Bound scalar command capability.
    pub const fn capability(&self) -> u64 {
        self.capability
    }
    /// Endpoint identifier checked against the trusted driver's descriptor.
    pub const fn endpoint(&self) -> u64 {
        self.endpoint
    }
    /// Fixed execution mode.
    pub const fn mode(&self) -> ExecutionMode {
        self.mode
    }
    /// Reservation binding key: `BindingKey::numeric(holder, capability,
    /// endpoint, mode)`. An accidental mix-up detector, not a credential.
    pub const fn reservation_key(&self) -> BindingKey {
        BindingKey::numeric(self.holder, self.capability, self.endpoint, self.mode)
    }
}

/// A trusted grant: a lease checked against one binding. Fields are private, the
/// type is neither `Clone` nor `Copy`, and it has an effect only when trusted
/// setup passes it to [`ActuatorAdapter::grant`].
///
/// ```compile_fail
/// use neuradix_embedded_core::DriverPermission;
/// let permission = DriverPermission {};
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::DriverPermission;
/// fn duplicate(permission: &DriverPermission) -> DriverPermission {
///     permission.clone()
/// }
/// ```
///
/// A reservation token backs at most one permission, and a plain generation (for
/// example one echoed in command metadata) cannot stand in for a token:
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorBinding, AuthorityLease, DriverPermission};
/// use neuradix_embedded_core::reservation::ReservedGeneration;
/// fn twice(b: ActuatorBinding, l1: AuthorityLease, l2: AuthorityLease, t: ReservedGeneration) {
///     let _first = DriverPermission::reserved(b, l1, t);
///     let _second = DriverPermission::reserved(b, l2, t);
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorBinding, AuthorityLease, CommandMeta, DriverPermission};
/// fn echoed(b: ActuatorBinding, l: AuthorityLease, meta: CommandMeta) {
///     let _forged = DriverPermission::reserved(b, l, meta.generation);
/// }
/// ```
///
/// ```
/// use neuradix_embedded_core::{ActuatorBinding, AuthorityLease, CommandMeta, DriverPermission, Generation};
/// fn echoed(b: ActuatorBinding, l: AuthorityLease, meta: CommandMeta) {
///     let _plain: Generation = meta.generation;
///     let _unreserved = DriverPermission::new(b, l);
/// }
/// ```
///
/// ```
/// use neuradix_embedded_core::{ActuatorBinding, AuthorityLease, DriverPermission};
/// use neuradix_embedded_core::reservation::ReservedGeneration;
/// fn once(b: ActuatorBinding, l1: AuthorityLease, l2: AuthorityLease, t: ReservedGeneration) {
///     let _first = DriverPermission::reserved(b, l1, t);
///     let _unreserved = DriverPermission::new(b, l2);
/// }
/// ```
#[derive(Debug)]
pub struct DriverPermission {
    binding: ActuatorBinding,
    lease: AuthorityLease,
    reserved: bool,
}
impl DriverPermission {
    /// Trusted setup only: require the lease holder/capability and the mode's
    /// timeline domain to match the binding. The generation is NOT proven
    /// reserved: use [`DriverPermission::reserved`] for live startup. Adapters
    /// built with [`ActuatorAdapter::new_reserved`], or that already accepted a
    /// reserved grant, refuse permissions made here.
    pub fn new(binding: ActuatorBinding, lease: AuthorityLease) -> Result<Self, PermissionError> {
        if lease.holder() != binding.holder || lease.capability() != binding.capability {
            return Err(PermissionError::BindingMismatch);
        }
        if lease.config().policy().timeline().domain() != binding.mode.clock_domain() {
            return Err(PermissionError::ModeMismatch);
        }
        Ok(Self {
            binding,
            lease,
            reserved: false,
        })
    }
    /// Trusted setup only: every [`new`](Self::new) check, then require the token
    /// to belong to this binding's reservation key and to carry exactly the
    /// lease's generation (`ReservationMismatch` otherwise). The token is consumed
    /// even on error, which burns its value harmlessly.
    ///
    /// The adapter holds no receiver identity: a token from another receiver's
    /// store with the same binding key is accepted. Trusted setup must open the
    /// reserver for its own receiver.
    pub fn reserved(
        binding: ActuatorBinding,
        lease: AuthorityLease,
        reservation: ReservedGeneration,
    ) -> Result<Self, PermissionError> {
        let mut permission = Self::new(binding, lease)?;
        if reservation.key().binding() != binding.reservation_key()
            || reservation.generation() != permission.generation()
        {
            return Err(PermissionError::ReservationMismatch);
        }
        permission.reserved = true;
        Ok(permission)
    }
    /// Trusted generation carried by this grant.
    pub fn generation(&self) -> Generation {
        self.lease.config().generation()
    }
    /// Whether this permission consumed a durable reservation token.
    pub fn is_reserved(&self) -> bool {
        self.reserved
    }
}

/// Configuration/control-plane failures. A failed call leaves state unchanged and
/// performs no driver I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionError {
    /// Safe output is not finite or not inside the hard limits.
    InvalidOutputConfig,
    /// Holder/capability/endpoint did not match the fixed binding.
    BindingMismatch,
    /// Driver, lease, port or control-time mode did not match.
    ModeMismatch,
    /// Duplicate/older generation or invalid renewal/control time.
    Session(ConfigError),
    /// A driver failure is latched; trusted reconstruction is required.
    DriverFault,
    /// Shutdown is terminal.
    Shutdown,
    /// Renewal cannot install a missing or revoked permission.
    NotGranted,
    /// The reservation token belongs to another binding key or carries a
    /// generation different from the lease's.
    ReservationMismatch,
    /// This adapter accepts only permissions backed by a reservation token.
    ReservationRequired,
}
impl core::fmt::Display for PermissionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for PermissionError {}

/// Bounded driver error vocabulary. Never infer physical state from an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// Device or bus unavailable.
    Unavailable,
    /// Driver refused the output.
    Rejected,
    /// The write may have taken effect; its outcome cannot be determined.
    UnknownOutcome,
}

/// Trusted board-support driver contract.
///
/// `write` must return within the deployment's admitted budget; this synchronous
/// API cannot bound an arbitrary blocking implementation. Endpoint and mode must
/// be fixed, and the owner must not keep another handle to the same output.
pub trait ActuatorDriver {
    /// Fixed endpoint descriptor, checked once before ownership is admitted.
    /// A descriptor is not hardware identity attestation.
    fn endpoint(&self) -> u64;
    /// Fixed execution mode of this driver implementation.
    fn mode(&self) -> ExecutionMode;
    /// Write a validated scalar. `Ok` acknowledges this call only; it is not
    /// evidence that equipment reached the value.
    fn write(&mut self, value: f32) -> Result<(), DriverError>;
}

/// Auditable permission disposition for one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// Installed trusted binding; the command gate still decides validity.
    Granted,
    /// No permission has been installed.
    Missing,
    /// Trusted revocation; a strictly newer generation is required.
    Revoked,
    /// The port's mode differs from the fixed endpoint mode.
    ModeMismatch,
    /// A driver error is latched across grants and later traffic.
    DriverFault,
    /// Explicit shutdown; no further driver calls occur.
    Shutdown,
    /// Installation/replacement established the safe output, awaiting input.
    Initialized,
}

/// Fixed-size, `Copy` record of one operation. Informational only: it is never
/// accepted as a dispatch credential.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchReport {
    /// Fixed binding, retained as evidence rather than authority.
    pub binding: ActuatorBinding,
    /// Current trusted generation, including a revoked watermark.
    pub generation: Option<Generation>,
    /// Trusted evaluation time supplied by setup, never a source timestamp.
    pub at: Timestamp,
    /// Why ingress was admitted or inhibited.
    pub permission: PermissionStatus,
    /// Gate evidence when a command or idle tick was evaluated normally.
    pub decision: Option<GateDecision>,
    /// Clock fault observed during inhibition; normal gate faults are in `decision`.
    pub evaluation_fault: Option<SafeReason>,
    /// Gate-selected value passed to the first driver call.
    pub output: f32,
    /// First driver call result; `None` means shutdown suppressed all I/O.
    pub write_result: Option<Result<(), DriverError>>,
    /// One immediate safe-output attempt after a failed first call. Never a retry
    /// of the requested output.
    pub fallback_result: Option<Result<(), DriverError>>,
    /// First driver fault, retained even when a later safe write succeeds.
    pub latched_driver_fault: Option<DriverError>,
}
impl DispatchReport {
    /// Number of driver calls made by this operation (0, 1 or 2).
    pub fn driver_calls(&self) -> usize {
        usize::from(self.write_result.is_some()) + usize::from(self.fallback_result.is_some())
    }
}

/// Owns one driver, one gate and one lease slot. Statically sized, no heap.
///
/// Not `Clone`/`Copy`; there is no driver or gate accessor and no dispatch
/// overload taking a [`GateDecision`] or raw scalar. Call a port periodically,
/// including with `None`, and shut down explicitly: `Drop` performs no I/O.
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorAdapter, ActuatorDriver};
/// fn escape<D: ActuatorDriver>(adapter: &mut ActuatorAdapter<D>) -> &mut D {
///     &mut adapter.driver
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorAdapter, ActuatorDriver};
/// fn fork<D: ActuatorDriver>(adapter: &ActuatorAdapter<D>) -> ActuatorAdapter<D> {
///     adapter.clone()
/// }
/// ```
pub struct ActuatorAdapter<D: ActuatorDriver> {
    binding: ActuatorBinding,
    driver: D,
    limits: Limits,
    safe: f32,
    /// Created by the first grant; holds the one lease slot and gate-wide clock.
    gate: Option<CommandGate>,
    /// Evaluation clock used before any grant; moved into the gate on first grant.
    pending_clock: EvaluationClock,
    status: PermissionStatus,
    driver_fault: Option<DriverError>,
    generation: Option<Generation>,
    /// Set by `new_reserved` or by the first successful reserved grant; never cleared.
    reservations_required: bool,
}
impl<D: ActuatorDriver> ActuatorAdapter<D> {
    /// Take exclusive driver ownership after validating its endpoint and mode and
    /// the safe output. Performs no I/O: setup must grant, or tick with `None`, to
    /// write the safe output before enabling equipment. On error the driver is
    /// dropped without any write.
    pub fn new(
        binding: ActuatorBinding,
        limits: Limits,
        safe_output: f32,
        driver: D,
    ) -> Result<Self, PermissionError> {
        if !safe_output.is_finite() || safe_output < limits.min() || safe_output > limits.max() {
            return Err(PermissionError::InvalidOutputConfig);
        }
        if driver.endpoint() != binding.endpoint {
            return Err(PermissionError::BindingMismatch);
        }
        if driver.mode() != binding.mode {
            return Err(PermissionError::ModeMismatch);
        }
        Ok(Self {
            binding,
            driver,
            limits,
            safe: safe_output,
            gate: None,
            pending_clock: EvaluationClock::default(),
            status: PermissionStatus::Missing,
            driver_fault: None,
            generation: None,
            reservations_required: false,
        })
    }

    /// Like [`new`](Self::new) (same checks, no I/O), but every grant must carry
    /// a permission made by [`DriverPermission::reserved`]. No API relaxes this.
    /// Use it for live startup.
    pub fn new_reserved(
        binding: ActuatorBinding,
        limits: Limits,
        safe_output: f32,
        driver: D,
    ) -> Result<Self, PermissionError> {
        let mut adapter = Self::new(binding, limits, safe_output, driver)?;
        adapter.reservations_required = true;
        Ok(adapter)
    }

    /// Whether unreserved permissions are refused: true for `new_reserved`, or
    /// after the first successful reserved grant (a one-way latch).
    pub fn requires_reserved_generations(&self) -> bool {
        self.reservations_required
    }

    /// Trusted installation or replacement. Requires the fixed binding, a
    /// control time in the mode's domain that does not regress the gate clock,
    /// and a strictly newer generation (also after revocation). Always attempts
    /// the safe output immediately; the next command is slewed from it.
    ///
    /// Check order: shutdown/driver fault, binding, mode, reservation
    /// requirement, control time, generation. A failure changes no state
    /// (including the reservation latch) and makes no driver call.
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
        if self.reservations_required && !permission.reserved {
            return Err(PermissionError::ReservationRequired);
        }
        self.clock()
            .check_control_time(now)
            .map_err(PermissionError::Session)?;
        let generation = permission.generation();
        let reserved = permission.reserved;
        match &mut self.gate {
            Some(gate) => gate
                .replace_lease(permission.lease)
                .map_err(PermissionError::Session)?,
            None => {
                let mut gate = CommandGate::new(self.limits, permission.lease, self.safe)
                    .map_err(|_| PermissionError::InvalidOutputConfig)?;
                *gate.clock_mut() = self.pending_clock;
                self.gate = Some(gate);
            }
        }
        self.reservations_required |= reserved;
        self.generation = Some(generation);
        self.status = PermissionStatus::Granted;
        Ok(self.inhibit(now, PermissionStatus::Initialized))
    }

    /// Trusted renewal of the active lease; preserves sequence, freshness,
    /// watchdog, slew and clock state. Performs no driver I/O.
    pub fn renew(&mut self, expires: Timestamp, now: Timestamp) -> Result<(), PermissionError> {
        self.control_ready()?;
        match (&mut self.gate, self.status) {
            (Some(gate), PermissionStatus::Granted) => gate
                .renew_lease(expires, now)
                .map_err(PermissionError::Session),
            _ => Err(PermissionError::NotGranted),
        }
    }

    /// Trusted revocation: immediately attempts the safe output and retains the
    /// generation watermark. After shutdown this performs no I/O.
    pub fn revoke(&mut self, now: Timestamp) -> DispatchReport {
        if self.status == PermissionStatus::Shutdown {
            return self.inhibit(now, PermissionStatus::Shutdown);
        }
        if let Some(gate) = &mut self.gate {
            gate.revoke_lease();
        }
        self.status = PermissionStatus::Revoked;
        self.inhibit(now, PermissionStatus::Revoked)
    }

    /// Terminal explicit shutdown. The first call makes at most two driver calls
    /// (safe write, one fallback on failure); later calls and ports make none.
    pub fn shutdown(&mut self, now: Timestamp) -> DispatchReport {
        if self.status == PermissionStatus::Shutdown {
            return self.inhibit(now, PermissionStatus::Shutdown);
        }
        let mut report = self.revoke(now);
        report.permission = PermissionStatus::Shutdown;
        self.status = PermissionStatus::Shutdown;
        report
    }

    /// Trusted scheduling creates a single-use port with immutable mode and
    /// runtime time. Give components only this port.
    pub fn port(&mut self, mode: ExecutionMode, now: Timestamp) -> ActuatorPort<'_, D> {
        ActuatorPort {
            adapter: self,
            mode,
            now,
        }
    }

    /// Fixed binding.
    pub fn binding(&self) -> ActuatorBinding {
        self.binding
    }
    /// Current permission state.
    pub fn status(&self) -> PermissionStatus {
        self.status
    }
    /// Current trusted generation, including a revoked watermark.
    pub fn generation(&self) -> Option<Generation> {
        self.generation
    }
    /// First driver failure, latched even if a safe write was acknowledged.
    pub fn driver_fault(&self) -> Option<DriverError> {
        self.driver_fault
    }
    /// Last command accepted by the gate, not last physical actuation.
    pub fn last_accepted_at(&self) -> Option<Timestamp> {
        self.gate.as_ref().and_then(CommandGate::last_accepted_at)
    }

    fn clock(&mut self) -> &mut EvaluationClock {
        match &mut self.gate {
            Some(gate) => gate.clock_mut(),
            None => &mut self.pending_clock,
        }
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
        input: Option<Command>,
    ) -> DispatchReport {
        let denied = if self.status == PermissionStatus::Shutdown {
            Some(PermissionStatus::Shutdown)
        } else if self.driver_fault.is_some() {
            Some(PermissionStatus::DriverFault)
        } else if mode != self.binding.mode {
            Some(PermissionStatus::ModeMismatch)
        } else if self.status != PermissionStatus::Granted {
            Some(self.status)
        } else {
            None
        };
        if let Some(reason) = denied {
            return self.inhibit(now, reason);
        }
        let Some(gate) = &mut self.gate else {
            return self.inhibit(now, PermissionStatus::Missing);
        };
        let decision = gate.evaluate(input, now);
        self.write_report(
            now,
            PermissionStatus::Granted,
            Some(decision),
            None,
            decision.applied,
        )
    }
    /// Select safe output for any inhibited path, observing runtime time without
    /// consuming a sequence or feeding the watchdog.
    fn inhibit(&mut self, now: Timestamp, reason: PermissionStatus) -> DispatchReport {
        let fault = match &mut self.gate {
            Some(gate) => gate.inhibit(now),
            None => self.pending_clock.observe(now).err(),
        };
        self.write_report(now, reason, None, fault, self.safe)
    }
    fn write_report(
        &mut self,
        now: Timestamp,
        permission: PermissionStatus,
        decision: Option<GateDecision>,
        evaluation_fault: Option<SafeReason>,
        output: f32,
    ) -> DispatchReport {
        let mut report = DispatchReport {
            binding: self.binding,
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
            // Never slew from an output whose write failed; never retry it.
            if let Some(gate) = &mut self.gate {
                gate.inhibit(now);
            }
            report.fallback_result = Some(self.driver.write(self.safe));
        }
        report.latched_driver_fault = self.driver_fault;
        report
    }
}

/// Single-use scoped ingress with mode and time fixed by trusted setup. It
/// exposes no grant, renewal, revocation, driver access or mode/time mutation.
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort, ExecutionMode};
/// fn escalate<D: ActuatorDriver>(port: &mut ActuatorPort<'_, D>) {
///     port.mode = ExecutionMode::Live;
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort};
/// use neuradix_time::Timestamp;
/// fn backdate<D: ActuatorDriver>(port: &mut ActuatorPort<'_, D>, source_at: Timestamp) {
///     port.now = source_at;
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort, GateDecision};
/// fn forged<D: ActuatorDriver>(port: ActuatorPort<'_, D>, decision: GateDecision) {
///     port.tick(Some(decision));
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort};
/// fn raw<D: ActuatorDriver>(port: ActuatorPort<'_, D>) {
///     port.tick(0.9_f32);
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort};
/// fn twice<D: ActuatorDriver>(port: ActuatorPort<'_, D>) {
///     port.tick(None);
///     port.tick(None);
/// }
/// ```
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort, DriverPermission};
/// use neuradix_time::Timestamp;
/// fn install<D: ActuatorDriver>(port: ActuatorPort<'_, D>, p: DriverPermission, now: Timestamp) {
///     port.grant(p, now);
/// }
/// ```
///
/// Components cannot reach the reservation allocator through a port either:
///
/// ```compile_fail
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort};
/// fn reserve<D: ActuatorDriver>(port: ActuatorPort<'_, D>) {
///     let _token = port.reserve();
/// }
/// ```
///
/// ```
/// use neuradix_embedded_core::{ActuatorDriver, ActuatorPort};
/// fn idle<D: ActuatorDriver>(port: ActuatorPort<'_, D>) {
///     let _report = port.tick(None);
/// }
/// ```
pub struct ActuatorPort<'a, D: ActuatorDriver> {
    adapter: &'a mut ActuatorAdapter<D>,
    mode: ExecutionMode,
    now: Timestamp,
}
impl<D: ActuatorDriver> ActuatorPort<'_, D> {
    /// Evaluate at the trusted runtime time and attempt the validated output.
    /// `None` is a periodic idle tick that enforces held-output expiry.
    pub fn tick(self, input: Option<Command>) -> DispatchReport {
        self.adapter.tick(self.mode, self.now, input)
    }
}
