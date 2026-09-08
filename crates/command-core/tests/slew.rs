use neuradix_command_core::SlewRate;
use neuradix_time::Duration;

#[test]
fn invalid_rates_and_arguments_cannot_produce_outputs() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
        assert!(SlewRate::new(bad).is_none());
    }
    let rate = SlewRate::new(1.0).unwrap();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(rate.apply_f64(bad, None).is_none());
        assert!(rate.apply_f64(0.0, Some((bad, Duration::ZERO))).is_none());
    }
    assert!(
        rate.apply_f64(1.0, Some((0.0, Duration::from_nanos(-1))))
            .is_none()
    );
    assert!(rate.apply_f32(f32::INFINITY, None).is_none());
}

#[test]
fn zero_elapsed_zero_rate_and_initialization_are_explicit() {
    for rate in [0.0, 1.0, f64::MAX] {
        let limit = SlewRate::new(rate).unwrap();
        assert_eq!(limit.apply_f64(7.0, None), Some(7.0));
        assert_eq!(limit.apply_f64(7.0, Some((2.0, Duration::ZERO))), Some(2.0));
        assert_eq!(limit.apply_f32(7.0, Some((2.0, Duration::ZERO))), Some(2.0));
    }
    assert_eq!(
        SlewRate::new(0.0)
            .unwrap()
            .apply_f64(7.0, Some((2.0, Duration::from_nanos(i128::MAX)))),
        Some(2.0)
    );
}

#[test]
fn arithmetic_overflow_rejects_instead_of_granting_unlimited_slew() {
    let rate = SlewRate::new(f64::MAX).unwrap();
    for prev in [-f64::MAX, 0.0, f64::MAX] {
        assert!(
            rate.apply_f64(1.0, Some((prev, Duration::from_secs(3))))
                .is_none()
        );
    }
    assert!(
        rate.apply_f32(1.0, Some((0.0, Duration::from_secs(3))))
            .is_none()
    );
    let tiny = SlewRate::new(f64::from_bits(1)).unwrap();
    assert_eq!(
        tiny.apply_f64(1.0, Some((0.0, Duration::from_nanos(1)))),
        Some(0.0)
    );
}

#[test]
fn binary32_rounding_is_inward_at_half_ulp_and_subnormal_boundaries() {
    for sign in [-1.0f32, 1.0] {
        let prev = sign;
        let next = if sign > 0.0 {
            prev.next_up()
        } else {
            prev.next_down()
        };
        let ulp = (next as f64 - prev as f64).abs();
        let rate = SlewRate::new(ulp * 0.75).unwrap();
        assert_eq!(
            rate.apply_f32(next, Some((prev, Duration::from_secs(1)))),
            Some(prev)
        );
        let rate = SlewRate::new(ulp).unwrap();
        assert_eq!(
            rate.apply_f32(next, Some((prev, Duration::from_secs(1)))),
            Some(next)
        );
    }
    let tiny = f32::from_bits(1);
    let rate = SlewRate::new(tiny as f64 * 0.75).unwrap();
    assert_eq!(
        rate.apply_f32(tiny, Some((0.0, Duration::from_secs(1)))),
        Some(0.0)
    );
}
