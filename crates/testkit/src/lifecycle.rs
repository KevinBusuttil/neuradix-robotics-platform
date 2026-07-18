//! Lifecycle transition helpers.

use neuradix_runtime::LifecycleState;

/// Assert that a direct transition `from -> to` is permitted.
///
/// # Panics
/// Panics if the transition is not permitted.
pub fn assert_legal(from: LifecycleState, to: LifecycleState) {
    assert!(
        from.can_transition_to(to),
        "expected `{from}` -> `{to}` to be a legal transition, but it was rejected"
    );
}

/// Assert that a direct transition `from -> to` is not permitted.
///
/// # Panics
/// Panics if the transition is unexpectedly permitted.
pub fn assert_illegal(from: LifecycleState, to: LifecycleState) {
    assert!(
        !from.can_transition_to(to),
        "expected `{from}` -> `{to}` to be an illegal transition, but it was allowed"
    );
}

/// Assert that a whole ordered sequence of states is pairwise legal.
///
/// # Panics
/// Panics on the first pair that is not a legal transition.
pub fn assert_legal_sequence(states: &[LifecycleState]) {
    for window in states.windows(2) {
        assert_legal(window[0], window[1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_recognise_legal_and_illegal_transitions() {
        assert_legal(LifecycleState::Declared, LifecycleState::Configured);
        assert_illegal(LifecycleState::Declared, LifecycleState::Active);
        assert_legal_sequence(&[
            LifecycleState::Declared,
            LifecycleState::Configured,
            LifecycleState::Inactive,
            LifecycleState::Active,
            LifecycleState::Stopping,
            LifecycleState::Stopped,
        ]);
    }
}
