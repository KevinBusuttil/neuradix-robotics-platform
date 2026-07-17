//! Tests for `neuradix-time`.

use neuradix_time::{Clock, ClockDomain, Duration, ManualClock, Timestamp};

#[test]
fn duration_parses_and_normalises() {
    assert_eq!(Duration::parse("100ms").unwrap().as_nanos(), 100_000_000);
    assert_eq!(Duration::parse("0.1s").unwrap().as_nanos(), 100_000_000);
    assert_eq!(
        Duration::parse("1s").unwrap(),
        Duration::parse("1000ms").unwrap()
    );
    assert_eq!(Duration::parse("2m").unwrap().as_nanos(), 120_000_000_000);
    assert_eq!(Duration::parse("-50us").unwrap().as_nanos(), -50_000);
    // A bare number without a unit is rejected.
    assert!(Duration::parse("100").is_none());
    assert!(Duration::parse("abc").is_none());
}

#[test]
fn manual_clock_is_deterministic_without_sleeping() {
    let clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
    let t0 = clock.now();
    clock.advance(Duration::from_millis(50)).unwrap();
    clock.advance(Duration::from_millis(50)).unwrap();
    let t1 = clock.now();
    assert_eq!(t1.duration_since(t0).unwrap(), Duration::from_millis(100));
    assert_eq!(t1.domain(), ClockDomain::Simulation);
}

#[test]
fn cross_domain_arithmetic_is_a_typed_error() {
    let a = Timestamp::new(ClockDomain::Monotonic, 10);
    let b = Timestamp::new(ClockDomain::Utc, 10);
    assert!(a.duration_since(b).is_err());
    assert!(a.compare(b).is_err());
    // Equality across domains is simply false, not an error.
    assert_ne!(a, b);
}

#[test]
fn timestamps_compare_within_a_domain() {
    let a = Timestamp::new(ClockDomain::Replay, 10);
    let b = Timestamp::new(ClockDomain::Replay, 20);
    assert_eq!(a.compare(b).unwrap(), std::cmp::Ordering::Less);
    assert_eq!(b.duration_since(a).unwrap(), Duration::from_nanos(10));
}

#[test]
fn clock_domain_byte_codes_round_trip_and_are_stable() {
    for (domain, expected) in [
        (ClockDomain::Monotonic, 0u8),
        (ClockDomain::Utc, 1),
        (ClockDomain::Sensor, 2),
        (ClockDomain::Simulation, 3),
        (ClockDomain::Replay, 4),
    ] {
        assert_eq!(domain.code(), expected);
        assert_eq!(ClockDomain::from_code(expected), Some(domain));
    }
    assert_eq!(ClockDomain::from_code(5), None);
}

#[test]
fn manual_clock_set_requires_matching_domain() {
    let clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
    assert!(
        clock
            .set(Timestamp::new(ClockDomain::Simulation, 500))
            .is_ok()
    );
    assert_eq!(clock.now().as_nanos(), 500);
    assert!(clock.set(Timestamp::new(ClockDomain::Utc, 0)).is_err());
}
