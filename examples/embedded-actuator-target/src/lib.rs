//! MCU cross-compilation evidence for the embedded actuator boundary.
//!
//! This `no_std` library links no allocator and monomorphizes
//! [`ActuatorAdapter`] with a concrete driver, so building it for a bare-metal
//! target (for example `thumbv6m-none-eabi`, `thumbv7em-none-eabihf` or
//! `riscv32imc-unknown-none-elf`) generates real target code for grant,
//! dispatch, fault handling and shutdown.
//!
//! [`DutyCycleDriver`] is a board-independent stand-in: it records the last
//! validated duty value in memory and touches no peripheral. A board package
//! would implement [`ActuatorDriver`] over its HAL; executor and transport stay
//! with the caller, which supplies trusted time and schedules [`control_step`].
//! Compiling this crate is not execution on hardware.
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use neuradix_embedded_core::{
    ActuatorAdapter, ActuatorBinding, ActuatorDriver, AuthorityLease, Command, DispatchReport,
    DriverError, DriverPermission, ExecutionMode, Limits, PermissionError,
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

/// Trusted setup: bind the owned driver. No I/O until a grant or tick.
pub fn setup(holder: u64, capability: u64) -> Result<Thruster, PermissionError> {
    ActuatorAdapter::new(
        ActuatorBinding::new(holder, capability, ENDPOINT, ExecutionMode::Live),
        Limits::with_slew_rate(-1.0, 1.0, 2.0).ok_or(PermissionError::InvalidOutputConfig)?,
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

/// Static RAM footprint of the adapter on the compiled target.
pub const ADAPTER_BYTES: usize = core::mem::size_of::<Thruster>();
/// Size of one fixed report on the compiled target.
pub const REPORT_BYTES: usize = core::mem::size_of::<DispatchReport>();
// Compile-time bound so a target build fails if footprint grows unexpectedly.
const _: () = assert!(ADAPTER_BYTES <= 640 && REPORT_BYTES <= 320);

/// Footprint `[adapter, report]` in bytes, emitted for target inspection.
pub static FOOTPRINT: [usize; 2] = [ADAPTER_BYTES, REPORT_BYTES];

#[cfg(test)]
mod tests {
    use super::*;
    use neuradix_embedded_core::{
        CommandMeta, CommandPolicy, Generation, PermissionStatus, SessionConfig, SharedTimeline,
    };
    use neuradix_time::{ClockDomain, Duration};

    fn t(ms: i128) -> Timestamp {
        Timestamp::new(ClockDomain::Monotonic, ms * 1_000_000)
    }

    #[test]
    fn target_shaped_sequence() {
        let mut adapter = setup(1, 2).unwrap();
        let policy = CommandPolicy::new(
            SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
            Duration::from_millis(200),
            Duration::ZERO,
            Duration::from_millis(50),
        )
        .unwrap();
        let g = Generation::new(1).unwrap();
        let lease = AuthorityLease::new(1, 2, SessionConfig::new(g, t(0), t(500), policy).unwrap());
        assert_eq!(
            install(&mut adapter, lease, t(0)).unwrap().permission,
            PermissionStatus::Initialized
        );
        let command = Command {
            holder: 1,
            capability: 2,
            value: 1.0,
            meta: CommandMeta {
                generation: g,
                sequence: 0,
                source_at: t(100),
                deadline: t(150),
                timeline: 1,
            },
        };
        let report = control_step(&mut adapter, t(100), Some(command));
        assert!((report.output - 0.2).abs() < 1e-6);
        assert_eq!(report.write_result, Some(Ok(())));
        assert_eq!(revoke(&mut adapter, t(110)).output, 0.0);
        assert_eq!(shutdown(&mut adapter, t(120)).driver_calls(), 1);
        assert_eq!(shutdown(&mut adapter, t(130)).driver_calls(), 0);
    }
}
