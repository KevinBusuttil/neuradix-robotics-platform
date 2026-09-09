//! Proves the headline claim end to end: a run recorded and replayed through a
//! fresh processor produces identical outputs — the *system* replays, not just
//! the data.

use neuradix_record::{
    Channel, NativeRecordWriter, NativeRecording, RecordCodec, RecordError, RecordingManifest,
};
use neuradix_runtime::{ComponentError, Processor, TickContext, run_lockstep};
use neuradix_time::{ClockDomain, ManualClock, Timestamp};

const CHANNEL: u16 = 0;

/// A stateful controller: emits a smoothed value tagged with the decision time,
/// so both the input value *and* the timestamp affect the output.
struct Smoother {
    previous: i64,
}

impl Processor for Smoother {
    type Input = i64;
    type Output = (i128, i64);

    fn process(
        &mut self,
        ctx: &TickContext,
        input: i64,
    ) -> Result<Vec<Self::Output>, ComponentError> {
        let smoothed = (self.previous + input) / 2;
        self.previous = smoothed;
        Ok(vec![(ctx.now.as_nanos(), smoothed)])
    }
}

/// Fixed-layout codec for an `i64` input.
struct I64Codec;

impl RecordCodec for I64Codec {
    type Message = i64;

    fn encode(&self, m: &i64) -> Vec<u8> {
        m.to_le_bytes().to_vec()
    }

    fn decode(&self, bytes: &[u8]) -> Result<i64, RecordError> {
        let arr: [u8; 8] = bytes
            .try_into()
            .map_err(|_| RecordError::Decode("expected 8 bytes".into()))?;
        Ok(i64::from_le_bytes(arr))
    }
}

fn inputs() -> Vec<(Timestamp, i64)> {
    [10, 20, 5, 40, 0, 100]
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            (
                Timestamp::new(ClockDomain::Simulation, i as i128 * 10_000),
                v,
            )
        })
        .collect()
}

#[test]
fn recorded_run_replays_to_identical_outputs() {
    let codec = I64Codec;

    // Live run.
    let live_clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
    let live_out = run_lockstep(&live_clock, &mut Smoother { previous: 0 }, inputs()).unwrap();

    // Record the inputs.
    let manifest = RecordingManifest::builder("lockstep-test")
        .channel(Channel {
            id: CHANNEL,
            name: "test/i64".to_owned(),
            schema_id: "sha256:test".to_owned(),
            clock_domain: "simulation".to_owned(),
        })
        .build();
    let mut writer = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    for (ts, value) in inputs() {
        writer
            .write_record(CHANNEL, 0, ts, &codec.encode(&value))
            .unwrap();
    }
    let bytes = writer.finish().unwrap();

    // Replay run: decode from the recording, fresh processor, replay clock.
    let recording = NativeRecording::from_bytes(&bytes).unwrap();
    let replay_inputs: Vec<(Timestamp, i64)> = recording
        .records_for(CHANNEL)
        .map(|r| (r.timestamp, codec.decode(&r.payload).unwrap()))
        .collect();
    let replay_clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
    let replay_out =
        run_lockstep(&replay_clock, &mut Smoother { previous: 0 }, replay_inputs).unwrap();

    assert_eq!(
        live_out, replay_out,
        "recorded run must replay to identical outputs"
    );
    assert_eq!(live_out.len(), 6);
}

struct ReplaySmoother(Smoother);
struct SmootherProgram;
impl neuradix_runtime::replay::ReplayProgram for SmootherProgram {
    type Instance = ReplaySmoother;
    fn identity(&self) -> &str {
        "test/smoother"
    }
    fn create(&self, configuration: &[u8], seed: u64) -> Result<ReplaySmoother, ComponentError> {
        if !configuration.is_empty() {
            return Err(ComponentError::Failed("unexpected config".into()));
        }
        let previous =
            i64::try_from(seed).map_err(|_| ComponentError::Failed("seed overflow".into()))?;
        Ok(ReplaySmoother(Smoother { previous }))
    }
}
fn output_bytes(value: &(i128, i64)) -> Vec<u8> {
    let mut bytes = value.0.to_le_bytes().to_vec();
    bytes.extend_from_slice(&value.1.to_le_bytes());
    bytes
}
impl Processor for ReplaySmoother {
    type Input = neuradix_runtime::replay::ReplayInput;
    type Output = Vec<u8>;
    fn process(
        &mut self,
        ctx: &TickContext,
        input: Self::Input,
    ) -> Result<Vec<Vec<u8>>, ComponentError> {
        let value = I64Codec
            .decode(&input.data)
            .map_err(|e| ComponentError::Failed(e.to_string()))?;
        assert_eq!(input.source_time.unwrap().domain(), ClockDomain::Simulation);
        assert_eq!(ctx.now.domain(), ClockDomain::Replay);
        Ok(self
            .0
            .process(ctx, value)?
            .iter()
            .map(output_bytes)
            .collect())
    }
}
#[test]
fn reusable_runner_executes_recorded_inputs_with_an_explicit_schedule() {
    use neuradix_runtime::replay::{
        ReplayCase, ReplayInput, ReplayLimits, ReplayOutcome, run_replay,
    };
    let clock = ManualClock::new(Timestamp::new(ClockDomain::Simulation, 0));
    let expected = run_lockstep(&clock, &mut Smoother { previous: 0 }, inputs()).unwrap();
    let manifest = RecordingManifest::builder("runner-test").build();
    let mut writer = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    for (source, value) in inputs() {
        writer
            .write_record(0, 0, source, &I64Codec.encode(&value))
            .unwrap();
    }
    let recording = NativeRecording::from_bytes(&writer.finish().unwrap()).unwrap();
    for seed in [0, 2] {
        let mut case = ReplayCase::new(
            "test/smoother",
            &[],
            seed,
            Timestamp::new(ClockDomain::Replay, 0),
            ReplayLimits::default(),
        )
        .unwrap();
        for (index, record) in recording.records().iter().enumerate() {
            // Fixture-defined execution schedule; no implicit domain conversion.
            let evaluation = Timestamp::new(ClockDomain::Replay, index as i128 * 10_000);
            case.push(
                evaluation,
                &ReplayInput {
                    data: record.payload.clone(),
                    source_time: Some(record.timestamp),
                },
                &[output_bytes(&expected[index])],
            )
            .unwrap();
        }
        let report = run_replay(&SmootherProgram, &case);
        assert_eq!(report.invoked, 6);
        assert_eq!(
            report.outcome,
            if seed == 0 {
                ReplayOutcome::Matched
            } else {
                ReplayOutcome::Mismatched
            }
        );
    }
}
