//! Deterministic, input-driven execution.
//!
//! A [`Processor`] is a unit of component logic: given the current time and an
//! input, it produces zero or more outputs. [`run_lockstep`] drives a processor
//! over a time-ordered input sequence under a [`ControllableClock`], setting the
//! clock to each input's timestamp before processing it.
//!
//! Because time is injected and inputs are ordered, the output is a pure
//! function of `(initial processor state, inputs)`. This is what makes a
//! recorded run replay identically: feeding the same inputs (decoded from a
//! recording) through a fresh processor under a replay clock reproduces the same
//! outputs — the *system* replays, not just the data.
//!
//! This increment implements the event/input-driven executor. A periodic
//! (rate-group) executor is a later addition that schedules ticks on a fixed
//! period; it shares this determinism model.

use neuradix_time::{ControllableClock, Timestamp};

use crate::error::ComponentError;

/// Context passed to a [`Processor`] on each input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickContext {
    /// The current time, as positioned by the executor for this input.
    pub now: Timestamp,
    /// The zero-based index of this input within the run.
    pub sequence: u64,
}

/// A unit of component processing logic.
///
/// Implementations MUST be deterministic: for a given initial state, the same
/// sequence of `(ctx, input)` calls must yield the same outputs.
pub trait Processor {
    /// The input type consumed on each step.
    type Input;
    /// The output type produced on each step.
    type Output;

    /// Process one input at `ctx.now`, returning any outputs.
    fn process(
        &mut self,
        ctx: &TickContext,
        input: Self::Input,
    ) -> Result<Vec<Self::Output>, ComponentError>;
}

/// Run `processor` over a time-ordered `inputs` sequence under `clock`.
///
/// For each `(timestamp, input)`, the clock is positioned at `timestamp` and the
/// processor is invoked; all outputs are collected in order. The clock domain of
/// every input timestamp must match `clock`'s domain, otherwise a typed error is
/// returned before any state changes for that input.
pub fn run_lockstep<C, P>(
    clock: &C,
    processor: &mut P,
    inputs: impl IntoIterator<Item = (Timestamp, P::Input)>,
) -> Result<Vec<P::Output>, ComponentError>
where
    C: ControllableClock,
    P: Processor,
{
    let mut outputs = Vec::new();
    for (index, (timestamp, input)) in inputs.into_iter().enumerate() {
        clock
            .set(timestamp)
            .map_err(|e| ComponentError::Failed(format!("clock control failed: {e}")))?;
        let ctx = TickContext {
            now: clock.now(),
            sequence: index as u64,
        };
        outputs.extend(processor.process(&ctx, input)?);
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuradix_time::{Clock, ClockDomain, ManualClock};

    /// A processor that emits a running total, tagged with the current time.
    struct Accumulator {
        total: i64,
    }

    impl Processor for Accumulator {
        type Input = i64;
        type Output = (i128, i64);

        fn process(
            &mut self,
            ctx: &TickContext,
            input: i64,
        ) -> Result<Vec<Self::Output>, ComponentError> {
            self.total += input;
            Ok(vec![(ctx.now.as_nanos(), self.total)])
        }
    }

    fn inputs() -> Vec<(Timestamp, i64)> {
        (0..5)
            .map(|i| {
                (
                    Timestamp::new(ClockDomain::Simulation, i as i128 * 1_000),
                    i,
                )
            })
            .collect()
    }

    #[test]
    fn lockstep_is_deterministic() {
        let clock_a = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
        let clock_b = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
        let out_a = run_lockstep(&clock_a, &mut Accumulator { total: 0 }, inputs()).unwrap();
        let out_b = run_lockstep(&clock_b, &mut Accumulator { total: 0 }, inputs()).unwrap();
        assert_eq!(out_a, out_b);
        assert_eq!(
            out_a,
            vec![(0, 0), (1_000, 1), (2_000, 3), (3_000, 6), (4_000, 10)]
        );
    }

    #[test]
    fn clock_positions_at_each_input_timestamp() {
        let clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
        let out = run_lockstep(&clock, &mut Accumulator { total: 0 }, inputs()).unwrap();
        // Final clock position is the last input's timestamp.
        assert_eq!(clock.now().as_nanos(), 4_000);
        assert_eq!(out.last().unwrap().0, 4_000);
    }

    #[test]
    fn domain_mismatch_is_reported() {
        let clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
        let bad = vec![(Timestamp::new(ClockDomain::Utc, 0), 1i64)];
        let err = run_lockstep(&clock, &mut Accumulator { total: 0 }, bad).unwrap_err();
        assert!(matches!(err, ComponentError::Failed(_)));
    }
}
