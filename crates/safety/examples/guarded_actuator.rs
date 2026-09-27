//! One host test driver. No physical device is operated or qualified.
use neuradix_safety::{
    AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest, Generation, Identity,
    SessionConfig, SharedTimeline, actuator::*,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};
struct TestDriver;
impl ActuatorDriver for TestDriver {
    fn endpoint(&self) -> &str {
        "example/thruster"
    }
    fn mode(&self) -> ExecutionMode {
        ExecutionMode::Live
    }
    fn write(&mut self, value: f64) -> Result<(), DriverError> {
        println!("test driver write: {value}");
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let t = |ms: i128| Timestamp::new(ClockDomain::Monotonic, ms * 1_000_000);
    let binding = ActuatorBinding::new(
        "controller",
        "thrust",
        "example/thruster",
        ExecutionMode::Live,
    )?;
    let mut adapter = ActuatorAdapter::new(
        ActuatorConfig::new(binding.clone(), -1.0, 1.0, 10.0, 0.0)?,
        TestDriver,
    )?;
    // A fixed generation is valid only for this isolated host example. Real startup
    // must durably reserve a newer generation before enabling ingress.
    let generation = Generation::new(1).unwrap();
    let command = |sequence: u64, ms: i128| {
        CommandRequest::new(
            Identity::new("controller"),
            Capability::new("thrust"),
            0.5,
            CommandMeta {
                generation,
                sequence,
                source_at: t(ms),
                deadline: t(ms + 500),
                timeline: 1,
            },
        )
    };
    assert_eq!(
        adapter
            .port(ExecutionMode::Live, t(0))
            .tick(Some(command(0, 0)))
            .permission,
        PermissionStatus::Missing
    );
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, ClockDomain::Monotonic)?,
        Duration::from_millis(500),
        Duration::ZERO,
        Duration::from_millis(100),
    )?;
    let session = SessionConfig::new(generation, t(0), t(1000), policy)?;
    let lease = AuthorityLease::new(
        Identity::new("controller"),
        Capability::new("thrust"),
        session,
        None,
    );
    adapter.grant(DriverPermission::new(binding, lease)?, t(0))?;
    let report = adapter
        .port(ExecutionMode::Live, t(100))
        .tick(Some(command(0, 100)));
    assert_eq!(report.output, 0.5);
    assert_eq!(report.write_result, Some(Ok(())));
    let denied = adapter
        .port(ExecutionMode::Replay, t(110))
        .tick(Some(command(1, 110)));
    assert_eq!(denied.permission, PermissionStatus::ModeMismatch);
    assert_eq!(denied.output, 0.0);
    let revoked = adapter.revoke(t(120));
    assert_eq!(revoked.output, 0.0);
    assert_eq!(
        adapter
            .port(ExecutionMode::Live, t(130))
            .tick(Some(command(1, 130)))
            .permission,
        PermissionStatus::Revoked
    );
    adapter.shutdown(t(140));
    println!("permission, mode isolation and immediate revocation verified on a host test driver");
    Ok(())
}
