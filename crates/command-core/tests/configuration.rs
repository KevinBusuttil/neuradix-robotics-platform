use neuradix_command_core::{CommandPolicy, CommandSession, ConfigError, Generation, SessionConfig, SharedTimeline};
use neuradix_time::{ClockDomain, Duration, Timestamp};
fn t(n: i128) -> Timestamp { Timestamp::new(ClockDomain::Monotonic, n) }
#[test]
fn invalid_configuration_cannot_construct_a_session() {
    assert!(Generation::new(0).is_none());
    assert!(SharedTimeline::new(0, ClockDomain::Monotonic).is_err());
    let timeline = SharedTimeline::new(1, ClockDomain::Monotonic).unwrap();
    for (age, skew, timeout) in [(0, 0, 1), (-1, 0, 1), (1, -1, 1), (1, 0, 0), (1, 0, -1)] {
        assert!(CommandPolicy::new(timeline, Duration::from_nanos(age), Duration::from_nanos(skew), Duration::from_nanos(timeout)).is_err());
    }
    let policy = CommandPolicy::new(timeline, Duration::from_nanos(1), Duration::ZERO, Duration::from_nanos(1)).unwrap();
    let gen1 = Generation::new(1).unwrap();
    assert!(SessionConfig::new(gen1, t(1), t(1), policy).is_err());
    assert!(SessionConfig::new(gen1, t(2), t(1), policy).is_err());
    assert!(SessionConfig::new(gen1, t(0), Timestamp::new(ClockDomain::Utc, 1), policy).is_err());
    let config = SessionConfig::new(gen1, t(0), t(2), policy).unwrap();
    let mut session = CommandSession::new(config);
    assert_eq!(session.replace(config), Err(ConfigError::ReusedGeneration));
    assert_eq!(session.renew(t(3), t(2)), Err(ConfigError::InactiveLease));
    assert_eq!(session.renew(t(1), t(0)), Err(ConfigError::InvalidLease));
    session.revoke();
    assert_eq!(session.renew(t(3), t(0)), Err(ConfigError::RevokedSession));
}
#[test]
fn generation_exhaustion_does_not_wrap_and_policy_is_read_only() {
    let timeline = SharedTimeline::new(1, ClockDomain::Monotonic).unwrap();
    let policy = CommandPolicy::new(timeline, Duration::from_nanos(i128::MAX), Duration::from_nanos(i128::MAX), Duration::from_nanos(i128::MAX)).unwrap();
    assert_eq!(policy.max_age(), Duration::from_nanos(i128::MAX));
    assert_eq!(policy.future_skew(), Duration::from_nanos(i128::MAX));
    assert_eq!(policy.watchdog_timeout(), Duration::from_nanos(i128::MAX));
    assert_eq!(policy.timeline(), timeline);
    let cfg = SessionConfig::new(Generation::new(u128::MAX).unwrap(), t(0), t(2), policy).unwrap();
    let mut session = CommandSession::new(cfg);
    let lower = SessionConfig::new(Generation::new(1).unwrap(), t(0), t(2), policy).unwrap();
    assert_eq!(session.replace(lower), Err(ConfigError::ReusedGeneration));
}
