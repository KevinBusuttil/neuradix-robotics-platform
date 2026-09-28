//! MCU cross-compilation evidence for the embedded actuator boundary.
//!
//! This `no_std` library links no allocator and monomorphizes
//! [`ActuatorAdapter`] with a concrete driver, so building it for a bare-metal
//! target (for example `thumbv6m-none-eabi`, `thumbv7em-none-eabihf` or
//! `riscv32imc-unknown-none-elf`) generates real target code for grant,
//! dispatch, fault handling and shutdown. It also compiles the reserved live
//! startup path: [`setup_reserved`] builds a strict adapter, [`open_reserver`]
//! opens a [`GenerationReserver`] over a two-slot store, [`reserve_and_install`]
//! commits a generation to both slots before granting it, and
//! [`replace_from_window`] replaces a granted lease with no store calls.
//!
//! [`DutyCycleDriver`] is a board-independent stand-in: it records the last
//! validated duty value in memory and touches no peripheral. [`VolatileTestSlots`]
//! is a RAM stand-in that is **NOT durable**: it exists for compile/size evidence
//! and tests only, and tokens from it are not reservations in the WP-A04.2 sense.
//! A board package would implement [`ActuatorDriver`] over its HAL and
//! [`ReservationStore`] over two independent erase units meeting contract C1–C8;
//! executor and transport stay with the caller, which supplies trusted time and
//! schedules [`control_step`]. Provisioning is not linked into this firmware
//! graph (only this crate's tests enable it). Compiling this crate is not
//! execution on hardware.
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use neuradix_embedded_core::reservation::{
    GenerationReserver, OpenError, OpenFailure, RECORD_BYTES, ReceiverId, ReservationKey,
    ReservationStore, ReserveError, ReservedGeneration, ReserverConfig, Slot, StoreError,
};
use neuradix_embedded_core::{
    ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, CommandPolicy,
    DispatchReport, DriverError, DriverPermission, ExecutionMode, Limits, PermissionError,
    SessionConfig, SessionError,
};
use neuradix_time::Timestamp;

/// Endpoint identifier of the stand-in duty-cycle output.
pub const ENDPOINT: u64 = 0x5057_4d30;

/// In-memory stand-in for a PWM output; records the last acknowledged value.
#[derive(Debug, Default)]
pub struct DutyCycleDriver {
    last: Option<f32>,
}
impl DutyCycleDriver {
    /// A driver with no recorded output.
    pub const fn new() -> Self {
        Self { last: None }
    }
}
impl ActuatorDriver for DutyCycleDriver {
    fn endpoint(&self) -> u64 {
        ENDPOINT
    }
    fn mode(&self) -> ExecutionMode {
        ExecutionMode::Live
    }
    fn write(&mut self, value: f32) -> Result<(), DriverError> {
        if !(-1.0..=1.0).contains(&value) {
            return Err(DriverError::Rejected);
        }
        self.last = Some(value);
        Ok(())
    }
}

/// Concrete adapter type compiled for the target.
pub type Thruster = ActuatorAdapter<DutyCycleDriver>;

const fn live_binding(holder: u64, capability: u64) -> ActuatorBinding {
    ActuatorBinding::new(holder, capability, ENDPOINT, ExecutionMode::Live)
}

fn limits() -> Result<Limits, PermissionError> {
    Limits::with_slew_rate(-1.0, 1.0, 2.0).ok_or(PermissionError::InvalidOutputConfig)
}

/// Trusted setup: bind the owned driver. No I/O until a grant or tick.
pub fn setup(holder: u64, capability: u64) -> Result<Thruster, PermissionError> {
    ActuatorAdapter::new(
        live_binding(holder, capability),
        limits()?,
        0.0,
        DutyCycleDriver::new(),
    )
}

/// Trusted setup: install or replace a permission; writes the safe output.
pub fn install(
    adapter: &mut Thruster,
    lease: AuthorityLease,
    now: Timestamp,
) -> Result<DispatchReport, PermissionError> {
    let permission = DriverPermission::new(adapter.binding(), lease)?;
    adapter.grant(permission, now)
}

/// One scheduled control step. The executor supplies trusted monotonic `now`;
/// the component-facing input is only an optional decoded command.
pub fn control_step(
    adapter: &mut Thruster,
    now: Timestamp,
    input: Option<Command>,
) -> DispatchReport {
    adapter.port(ExecutionMode::Live, now).tick(input)
}

/// Trusted revocation.
pub fn revoke(adapter: &mut Thruster, now: Timestamp) -> DispatchReport {
    adapter.revoke(now)
}

/// Terminal shutdown; must be called explicitly before the adapter is dropped.
pub fn shutdown(adapter: &mut Thruster, now: Timestamp) -> DispatchReport {
    adapter.shutdown(now)
}

/// Volatile RAM slot pair. **NOT DURABLE**: compile/size evidence and tests
/// only; never a board store. A reset loses both slots, so tokens from it are not
/// reservations in the WP-A04.2 sense. Deliberately not `Clone` (contract C5).
pub struct VolatileTestSlots {
    slots: [[u8; RECORD_BYTES]; 2],
}
impl VolatileTestSlots {
    /// Both slots erased (all `0xFF`); `open` refuses it as `Blank`.
    pub const fn erased() -> Self {
        Self {
            slots: [[0xFF; RECORD_BYTES]; 2],
        }
    }
}
impl ReservationStore for VolatileTestSlots {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        #[cfg(test)]
        tests::count_store_call(false);
        *buf = self.slots[slot.index()];
        Ok(())
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        #[cfg(test)]
        tests::count_store_call(true);
        self.slots[slot.index()] = *record;
        Ok(())
    }
}

/// Zero-sized store used only to measure the reserver's own footprint.
struct NullSlots;
impl ReservationStore for NullSlots {
    fn read(&mut self, _: Slot, _: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    fn write(&mut self, _: Slot, _: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
}

/// Reserver over borrowed volatile test slots, as compiled for the target.
pub type Reserver<'s> = GenerationReserver<&'s mut VolatileTestSlots>;

/// Reservation key for `binding` on this receiver. The receiver identity must
/// come from outside the reservation storage image (MCU UID or OTP).
pub const fn reservation_key(receiver: ReceiverId, binding: ActuatorBinding) -> ReservationKey {
    ReservationKey::new(receiver, binding.reservation_key())
}

/// Trusted live setup: like [`setup`], but every grant must carry a permission
/// made from a reservation token. No I/O until a grant or tick.
pub fn setup_reserved(holder: u64, capability: u64) -> Result<Thruster, PermissionError> {
    ActuatorAdapter::new_reserved(
        live_binding(holder, capability),
        limits()?,
        0.0,
        DutyCycleDriver::new(),
    )
}

/// Trusted startup: open the reserver. Exactly two reads, never a write. A
/// refusal returns the borrowed slots with the typed error; the caller keeps
/// ticking the adapter at its safe output.
pub fn open_reserver(
    slots: &mut VolatileTestSlots,
    key: ReservationKey,
    config: ReserverConfig,
) -> Result<Reserver<'_>, OpenFailure<&mut VolatileTestSlots>> {
    GenerationReserver::open(slots, key, config)
}

/// Trusted lease parameters; the generation always comes from a reservation token.
#[derive(Debug, Clone, Copy)]
pub struct LeaseTemplate {
    /// Lease issue time, inclusive.
    pub issued: Timestamp,
    /// Lease expiry, exclusive.
    pub expires: Timestamp,
    /// Command validity policy.
    pub policy: CommandPolicy,
}

/// Why reserved startup or replacement did not grant. Never holds a store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupError {
    /// `open` refused; the slots were returned in the `OpenFailure`.
    Open(OpenError),
    /// No value was issued (includes `CommitRequired` from the window path).
    Reserve(ReserveError),
    /// The lease template is invalid for the reserved generation.
    Session(SessionError),
    /// The permission or grant was refused; the token was burned.
    Permission(PermissionError),
}
impl core::fmt::Display for StartupError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for StartupError {}

/// Trusted startup, post-revoke replacement or recovery: reserve a generation,
/// committing it to both slots first when the window is empty (always on the
/// first call after `open`), then grant it, which writes the safe output. A
/// commit may block for erase/program time: call this before ingress or after
/// [`revoke`], never from a context that must not delay evaluation ticks.
pub fn reserve_and_install(
    adapter: &mut Thruster,
    reserver: &mut Reserver<'_>,
    template: LeaseTemplate,
    now: Timestamp,
) -> Result<DispatchReport, StartupError> {
    let token = reserver.reserve().map_err(StartupError::Reserve)?;
    grant_reserved(adapter, token, template, now)
}

/// Replacement while granted: take the next value from the already committed
/// window with zero store calls, then grant it. An empty window returns
/// `StartupError::Reserve(ReserveError::CommitRequired)` and changes nothing;
/// revoke first, then use [`reserve_and_install`].
pub fn replace_from_window(
    adapter: &mut Thruster,
    reserver: &mut Reserver<'_>,
    template: LeaseTemplate,
    now: Timestamp,
) -> Result<DispatchReport, StartupError> {
    let token = reserver
        .reserve_from_window()
        .map_err(StartupError::Reserve)?;
    grant_reserved(adapter, token, template, now)
}

fn grant_reserved(
    adapter: &mut Thruster,
    token: ReservedGeneration,
    template: LeaseTemplate,
    now: Timestamp,
) -> Result<DispatchReport, StartupError> {
    let binding = adapter.binding();
    let config = SessionConfig::new(
        token.generation(),
        template.issued,
        template.expires,
        template.policy,
    )
    .map_err(StartupError::Session)?;
    let lease = AuthorityLease::new(binding.holder(), binding.capability(), config);
    let permission =
        DriverPermission::reserved(binding, lease, token).map_err(StartupError::Permission)?;
    adapter
        .grant(permission, now)
        .map_err(StartupError::Permission)
}

/// Test-only trusted commissioning of the volatile slots with a registry epoch.
/// The firmware build has no `provision` function at all.
#[cfg(test)]
fn commission(
    slots: &mut VolatileTestSlots,
    key: ReservationKey,
    epoch: neuradix_embedded_core::reservation::NamespaceEpoch,
) -> Result<
    neuradix_embedded_core::reservation::ProvisionReport,
    neuradix_embedded_core::reservation::ProvisionError,
> {
    neuradix_embedded_core::reservation::provision(
        slots,
        key,
        epoch,
        neuradix_embedded_core::reservation::ProvisionGuards::default(),
    )
}

/// Static RAM footprint of the adapter on the compiled target.
pub const ADAPTER_BYTES: usize = core::mem::size_of::<Thruster>();
/// Size of one fixed report on the compiled target.
pub const REPORT_BYTES: usize = core::mem::size_of::<DispatchReport>();
/// Reserver state on the compiled target, excluding its store handle.
pub const RESERVER_BYTES: usize = core::mem::size_of::<GenerationReserver<NullSlots>>();
/// Size of one reservation token on the compiled target.
pub const TOKEN_BYTES: usize = core::mem::size_of::<ReservedGeneration>();
// Compile-time bounds so a target build fails if footprint grows unexpectedly.
const _: () = assert!(ADAPTER_BYTES <= 640 && REPORT_BYTES <= 320);
const _: () = assert!(RESERVER_BYTES <= 128 && TOKEN_BYTES <= 64);

/// Footprint `[adapter, report, reserver, token]` in bytes, emitted for target
/// inspection.
pub static FOOTPRINT: [usize; 4] = [ADAPTER_BYTES, REPORT_BYTES, RESERVER_BYTES, TOKEN_BYTES];

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use core::num::NonZeroU32;
    use neuradix_embedded_core::reservation::{
        BindingKey, NamespaceEpoch, ProvisionReport, Remedy, RollbackDefense, RollbackPosture,
        SlotCondition,
    };
    use neuradix_embedded_core::{
        CommandMeta, CommandPolicy, Generation, Outcome, PermissionStatus, SafeReason,
        SessionConfig, SharedTimeline,
    };
    use neuradix_time::{ClockDomain, Duration};

    std::thread_local! {
        /// `(reads, writes)` made on `VolatileTestSlots` by this test thread.
        static STORE_CALLS: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
    }

    pub(super) fn count_store_call(write: bool) {
        STORE_CALLS.with(|calls| {
            let (reads, writes) = calls.get();
            calls.set(if write {
                (reads, writes + 1)
            } else {
                (reads + 1, writes)
            });
        });
    }

    /// Store calls since the previous call to `take_calls`.
    fn take_calls() -> (u32, u32) {
        STORE_CALLS.with(|calls| calls.replace((0, 0)))
    }

    fn t(ms: i128) -> Timestamp {
        Timestamp::new(ClockDomain::Monotonic, ms * 1_000_000)
    }

    fn policy() -> CommandPolicy {
        CommandPolicy::new(
            SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
            Duration::from_millis(200),
            Duration::ZERO,
            Duration::from_millis(50),
        )
        .unwrap()
    }

    fn command(generation: Generation, sequence: u64, ms: i128) -> Command {
        Command {
            holder: 1,
            capability: 2,
            value: 1.0,
            meta: CommandMeta {
                generation,
                sequence,
                source_at: t(ms),
                deadline: t(ms + 50),
                timeline: 1,
            },
        }
    }

    fn reserved(counter: u64) -> Generation {
        Generation::new((1u128 << 64) | u128::from(counter)).unwrap()
    }

    #[test]
    fn target_shaped_sequence() {
        let mut adapter = setup(1, 2).unwrap();
        let g = Generation::new(1).unwrap();
        let lease =
            AuthorityLease::new(1, 2, SessionConfig::new(g, t(0), t(500), policy()).unwrap());
        assert_eq!(
            install(&mut adapter, lease, t(0)).unwrap().permission,
            PermissionStatus::Initialized
        );
        let report = control_step(&mut adapter, t(100), Some(command(g, 0, 100)));
        assert!((report.output - 0.2).abs() < 1e-6);
        assert_eq!(report.write_result, Some(Ok(())));
        assert_eq!(revoke(&mut adapter, t(110)).output, 0.0);
        assert_eq!(shutdown(&mut adapter, t(120)).driver_calls(), 1);
        assert_eq!(shutdown(&mut adapter, t(130)).driver_calls(), 0);
    }

    #[test]
    fn t1_reserved_target_sequence() {
        let template = LeaseTemplate {
            issued: t(0),
            expires: t(10_000),
            policy: policy(),
        };
        let receiver = ReceiverId::new([0x5A; 16]).unwrap();
        let mut adapter = setup_reserved(1, 2).unwrap();
        assert!(adapter.requires_reserved_generations());
        let key = reservation_key(receiver, adapter.binding());
        assert_eq!(key.receiver(), receiver);
        assert_eq!(
            key.binding(),
            BindingKey::numeric(1, 2, ENDPOINT, ExecutionMode::Live)
        );
        let config = ReserverConfig::new(NonZeroU32::new(2).unwrap(), RollbackDefense::Unprotected);
        let mut slots = VolatileTestSlots::erased();

        // Before any grant the strict adapter ticks to the safe output.
        let idle = control_step(&mut adapter, t(0), None);
        assert_eq!(idle.permission, PermissionStatus::Missing);
        assert_eq!(idle.output, 0.0);
        assert_eq!(take_calls(), (0, 0));

        // Uncommissioned slots fail closed: two reads, no write, slots returned.
        let refused = open_reserver(&mut slots, key, config)
            .map_err(|failure| StartupError::Open(failure.error))
            .unwrap_err();
        assert_eq!(refused, StartupError::Open(OpenError::Blank));
        assert_eq!(OpenError::Blank.remedy(), Remedy::Provision);
        assert_eq!(take_calls(), (2, 0));

        // Test-only commissioning: 2 reads + 2 verified writes.
        let epoch = NamespaceEpoch::new(1).unwrap();
        assert_eq!(
            commission(&mut slots, key, epoch).unwrap(),
            ProvisionReport {
                epoch,
                replaced: None,
                commits: 1
            }
        );
        assert_eq!(take_calls(), (4, 2));

        let mut reserver = open_reserver(&mut slots, key, config)
            .map_err(|failure| failure.error)
            .unwrap();
        assert_eq!(take_calls(), (2, 0));
        let status = reserver.status();
        assert_eq!(status.durable_ceiling, 1u128 << 64);
        assert_eq!(status.window_remaining, 0);
        assert_eq!(status.rollback, RollbackPosture::Unprotected);
        assert_eq!(status.poisoned, None);

        // An unreserved install on the strict adapter is refused before any I/O,
        // even with a generation above every reserved value.
        let plain = AuthorityLease::new(
            1,
            2,
            SessionConfig::new(reserved(1_000), t(0), t(500), policy()).unwrap(),
        );
        assert_eq!(
            install(&mut adapter, plain, t(0)).unwrap_err(),
            PermissionError::ReservationRequired
        );
        assert_eq!(adapter.status(), PermissionStatus::Missing);
        assert_eq!(adapter.generation(), None);
        assert_eq!(take_calls(), (0, 0));

        // Startup: reserve commits to both slots (2 reads, 2 writes, 2 read-backs),
        // then the grant writes the safe output.
        let report = reserve_and_install(&mut adapter, &mut reserver, template, t(0)).unwrap();
        assert_eq!(take_calls(), (4, 2));
        let g1 = reserved(1);
        assert_eq!(report.permission, PermissionStatus::Initialized);
        assert_eq!(report.generation, Some(g1));
        assert_eq!(report.output, 0.0);
        assert_eq!(report.write_result, Some(Ok(())));
        assert_eq!(report.driver_calls(), 1);
        assert_eq!(adapter.status(), PermissionStatus::Granted);
        let status = reserver.status();
        assert_eq!(status.durable_ceiling, (1u128 << 64) | 2);
        assert_eq!(status.window_remaining, 1);
        assert_eq!(status.commits, 2);
        assert_eq!(status.slots, [SlotCondition::Current; 2]);

        let report = control_step(&mut adapter, t(100), Some(command(g1, 0, 100)));
        assert_eq!(report.permission, PermissionStatus::Granted);
        assert!((report.output - 0.2).abs() < 1e-6);
        assert_eq!(report.write_result, Some(Ok(())));

        // Still strict after a reserved grant: unreserved replacement is refused
        // and changes nothing.
        let plain = AuthorityLease::new(
            1,
            2,
            SessionConfig::new(reserved(1_000), t(0), t(500), policy()).unwrap(),
        );
        assert_eq!(
            install(&mut adapter, plain, t(110)).unwrap_err(),
            PermissionError::ReservationRequired
        );
        assert_eq!(adapter.generation(), Some(g1));
        assert_eq!(adapter.status(), PermissionStatus::Granted);

        // Replacement while granted: from the committed window, zero store calls.
        // Neither the refused installs nor the ticks above touched the store.
        assert_eq!(take_calls(), (0, 0));
        let report = replace_from_window(&mut adapter, &mut reserver, template, t(120)).unwrap();
        assert_eq!(take_calls(), (0, 0));
        let g2 = reserved(2);
        assert!(g2 > g1);
        assert_eq!(report.permission, PermissionStatus::Initialized);
        assert_eq!(report.generation, Some(g2));
        assert_eq!(report.output, 0.0);
        assert_eq!(report.write_result, Some(Ok(())));
        assert_eq!(reserver.status().window_remaining, 0);

        // The replaced generation's commands drive only the safe output.
        let stale = control_step(&mut adapter, t(130), Some(command(g1, 1, 130)));
        assert_eq!(
            stale.decision.map(|decision| decision.outcome),
            Some(Outcome::SafeState(SafeReason::GenerationMismatch))
        );
        assert_eq!(stale.output, 0.0);

        // An empty window refuses without store calls, poisoning or state change.
        let refused =
            replace_from_window(&mut adapter, &mut reserver, template, t(140)).unwrap_err();
        assert_eq!(refused, StartupError::Reserve(ReserveError::CommitRequired));
        assert_eq!(
            ReserveError::CommitRequired.remedy(),
            Remedy::CommitWhenSafe
        );
        assert_eq!(take_calls(), (0, 0));
        assert_eq!(reserver.status().poisoned, None);
        assert_eq!(reserver.status().durable_ceiling, (1u128 << 64) | 2);
        assert_eq!(adapter.generation(), Some(g2));
        assert_eq!(adapter.status(), PermissionStatus::Granted);

        // Commit when safe: revoke (safe output) first, then reserve commits.
        assert_eq!(revoke(&mut adapter, t(150)).output, 0.0);
        let report = reserve_and_install(&mut adapter, &mut reserver, template, t(160)).unwrap();
        assert_eq!(take_calls(), (4, 2));
        let g3 = reserved(3);
        assert!(g3 > g2);
        assert_eq!(report.generation, Some(g3));
        assert_eq!(report.permission, PermissionStatus::Initialized);
        let report = control_step(&mut adapter, t(170), Some(command(g3, 0, 170)));
        assert!((report.output - 0.02).abs() < 1e-6);

        // Restart over the same slots: the unused window value is skipped and the
        // first reserve commits again.
        let _released = reserver.into_store();
        assert_eq!(shutdown(&mut adapter, t(180)).driver_calls(), 1);
        let mut adapter = setup_reserved(1, 2).unwrap();
        let mut reserver = open_reserver(&mut slots, key, config)
            .map_err(|failure| failure.error)
            .unwrap();
        let report = reserve_and_install(&mut adapter, &mut reserver, template, t(0)).unwrap();
        assert_eq!(take_calls(), (2 + 4, 2));
        assert_eq!(report.generation, Some(reserved(5)));
        assert!(reserved(5) > g3);
    }

    #[test]
    fn footprint_matches_and_null_slots_fail_closed() {
        assert_eq!(
            FOOTPRINT,
            [ADAPTER_BYTES, REPORT_BYTES, RESERVER_BYTES, TOKEN_BYTES]
        );
        let receiver = ReceiverId::new([1; 16]).unwrap();
        let key = reservation_key(receiver, live_binding(1, 2));
        let config = ReserverConfig::new(NonZeroU32::MIN, RollbackDefense::Unprotected);
        let failure = GenerationReserver::open(NullSlots, key, config).unwrap_err();
        assert_eq!(failure.error, OpenError::StoreUnavailable);
    }
}
