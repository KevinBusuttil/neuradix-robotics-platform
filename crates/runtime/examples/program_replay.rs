//! Run two compiled controllers over the same admitted recording-derived case.
use neuradix_runtime::replay::*;
use neuradix_runtime::{ComponentError, Processor, TickContext};
use neuradix_time::{ClockDomain, Timestamp};

struct Program<const BIAS: i64>;
struct Sum<const BIAS: i64> {
    total: i64,
    gain: i64,
}
impl<const BIAS: i64> ReplayProgram for Program<BIAS> {
    type Instance = Sum<BIAS>;
    fn identity(&self) -> &str {
        "example/sum"
    }
    fn create(&self, config: &[u8], seed: u64) -> Result<Self::Instance, ComponentError> {
        if config.len() != 1 {
            return Err(ComponentError::Failed("one gain byte required".into()));
        }
        Ok(Sum {
            total: i64::try_from(seed)
                .map_err(|_| ComponentError::Failed("seed out of range".into()))?,
            gain: i64::from(config[0]),
        })
    }
}
impl<const BIAS: i64> Processor for Sum<BIAS> {
    type Input = ReplayInput;
    type Output = Vec<u8>;
    fn process(
        &mut self,
        _ctx: &TickContext,
        input: ReplayInput,
    ) -> Result<Vec<Vec<u8>>, ComponentError> {
        let fail = || ComponentError::Failed("invalid input or arithmetic overflow".into());
        let value = i64::from_le_bytes(input.data.as_slice().try_into().map_err(|_| fail())?);
        self.total = value
            .checked_mul(self.gain)
            .and_then(|v| v.checked_add(BIAS))
            .and_then(|v| self.total.checked_add(v))
            .ok_or_else(fail)?;
        Ok(vec![self.total.to_le_bytes().to_vec()])
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut case = ReplayCase::new(
        "example/sum",
        &[1],
        0,
        Timestamp::new(ClockDomain::Replay, 0),
        ReplayLimits::default(),
    )?;
    // A recording adapter must supply a deliberate evaluation schedule separately
    // from source timestamps. These source clocks never drive TickContext.now.
    for (index, (value, expected)) in [(2i64, 2i64), (3, 5), (4, 9)].into_iter().enumerate() {
        case.push(
            Timestamp::new(ClockDomain::Replay, index as i128),
            &ReplayInput {
                data: value.to_le_bytes().to_vec(),
                source_time: Some(Timestamp::new(ClockDomain::Sensor, 99)),
            },
            &[expected.to_le_bytes().to_vec()],
        )?;
    }
    let unchanged = run_replay(&Program::<0>, &case);
    let changed = run_replay(&Program::<1>, &case);
    assert_eq!(unchanged.outcome, ReplayOutcome::Matched);
    assert_eq!(changed.outcome, ReplayOutcome::Mismatched);
    assert_eq!(unchanged.invoked, 3);
    assert_eq!(changed.invoked, 3);
    println!(
        "unchanged: {:?}, invoked={}; changed: {:?}, invoked={}, differences={}",
        unchanged.outcome, unchanged.invoked, changed.outcome, changed.invoked, changed.differences
    );
    Ok(())
}
