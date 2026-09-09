//! Bounded single-processor replay with exact, per-tick byte comparison.
//!
//! This host API invokes trusted in-process code, not an executable sandbox.
//! Factories must return fresh isolated state and use only supplied configuration,
//! seed and TickContext time. Arbitrary factories/processors/Drop may allocate or
//! block: run outside local control under external supervision. Returned output
//! batches are checked before comparison, but Processor allocates them itself.
//! No resource or wall-time bound on arbitrary component code is claimed.
use crate::{ComponentError, Processor, TickContext};
use neuradix_time::{Clock, ControllableClock, ManualClock, Timestamp};

/// Admission errors; a rejected insertion leaves the case unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReplayError {
    /// Invalid policy or empty/oversized declared identity.
    #[error("invalid replay configuration")]
    Configuration,
    /// Inclusive count, payload, case or report budget exceeded.
    #[error("replay budget exceeded")]
    Limit,
    /// Evaluation time has an unsupported domain relationship.
    #[error("replay evaluation clock mismatch")]
    ClockMismatch,
    /// Evaluation time regresses from the start or previous tick.
    #[error("replay evaluation time regressed")]
    ClockRegression,
    /// Adjacent evaluation elapsed time cannot fit i128 nanoseconds.
    #[error("replay elapsed time overflow")]
    ClockOverflow,
}

/// Private validated inclusive policy. Payload/case bytes are logical owned data;
/// case accounting includes conservative collection overhead. Report bytes cover
/// the diagnostic array, excluding the fixed report object and borrowed case.
#[derive(Debug, Clone, Copy)]
pub struct ReplayLimits {
    steps: usize, outputs: usize, payload: usize, case_bytes: usize,
    diagnostics: usize, report_bytes: usize,
}
impl ReplayLimits {
    /// Validate counts (1..=1M), payload (1..=16 MiB), case (1 KiB..=256 MiB),
    /// diagnostic count (0..=4096) and diagnostic storage (0..=1 MiB).
    pub fn new(steps:usize,outputs:usize,payload:usize,case_bytes:usize,diagnostics:usize,report_bytes:usize) -> Result<Self,ReplayError> {
        if !(1..=1_000_000).contains(&steps) || !(1..=1_000_000).contains(&outputs)
            || !(1..=16<<20).contains(&payload) || !(1024..=256<<20).contains(&case_bytes)
            || diagnostics>4096 || report_bytes>1<<20
            || diagnostics.checked_mul(std::mem::size_of::<ReplayMismatch>()).is_none_or(|n|n>report_bytes) {
            return Err(ReplayError::Configuration);
        }
        Ok(Self {steps,outputs,payload,case_bytes,diagnostics,report_bytes})
    }
}
impl Default for ReplayLimits {
    fn default() -> Self { Self::new(10_000,100_000,1<<20,64<<20,64,16<<10).unwrap() }
}

/// Source data passed to the processor. Source time never schedules execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayInput {
    /// Opaque input bytes. The selected processor validates/decodes its format.
    pub data: Vec<u8>,
    /// Original source timestamp/provenance, or unknown. Other domains are allowed.
    pub source_time: Option<Timestamp>,
}
#[derive(Debug)]
struct Step { evaluation:Timestamp,input:ReplayInput,expected:Box<[Vec<u8>]> }

/// Owned admitted case; configuration, schedule and expectations cannot be mutated
/// through getters. Build with checked push; run borrows the completed case.
///
/// ```compile_fail
/// let policy = neuradix_runtime::replay::ReplayLimits::default();
/// policy.steps = usize::MAX;
/// ```
#[derive(Debug)]
pub struct ReplayCase {
    identity:String, configuration:Vec<u8>, seed:u64, start:Timestamp,
    limits:ReplayLimits, steps:Vec<Step>, expected_count:usize, accounted:usize,
}
impl ReplayCase {
    /// Snapshot a nonempty declared identity (<=256 UTF-8 bytes), configuration
    /// (<=payload limit), seed and initial trusted replay evaluation time.
    pub fn new(identity:&str,configuration:&[u8],seed:u64,start:Timestamp,limits:ReplayLimits) -> Result<Self,ReplayError> {
        if identity.is_empty() || identity.len()>256 { return Err(ReplayError::Configuration); }
        let accounted=add(add(512,identity.len())?,configuration.len())?;
        if configuration.len()>limits.payload || accounted>limits.case_bytes { return Err(ReplayError::Limit); }
        Ok(Self {identity:identity.into(),configuration:configuration.to_vec(),seed,start,limits,steps:Vec::new(),expected_count:0,accounted})
    }
    /// Add one evaluation/input and its ordered expected output batch. All budgets
    /// and time checks precede copying; failure leaves the previous case usable.
    /// Equal evaluation times preserve insertion order. Negative epochs are valid.
    pub fn push(&mut self,evaluation:Timestamp,input:&ReplayInput,expected:&[Vec<u8>]) -> Result<(),ReplayError> {
        let previous=self.steps.last().map_or(self.start,|s|s.evaluation);
        if evaluation.domain()!=self.start.domain() { return Err(ReplayError::ClockMismatch); }
        if evaluation.as_nanos()<previous.as_nanos() { return Err(ReplayError::ClockRegression); }
        evaluation.duration_since(previous).map_err(|_|ReplayError::ClockOverflow)?;
        let count=add(self.expected_count,expected.len())?;
        if self.steps.len()>=self.limits.steps || count>self.limits.outputs || input.data.len()>self.limits.payload { return Err(ReplayError::Limit); }
        let mut charge=add(256,input.data.len())?;
        for value in expected {
            if value.len()>self.limits.payload { return Err(ReplayError::Limit); }
            charge=add(charge,add(64,value.len())?)?;
        }
        let accounted=add(self.accounted,charge)?;
        if accounted>self.limits.case_bytes { return Err(ReplayError::Limit); }
        self.steps.push(Step {evaluation,input:input.clone(),expected:expected.to_vec().into_boxed_slice()});
        self.expected_count=count; self.accounted=accounted;
        Ok(())
    }
    /// Caller-declared program/component label, not binary authentication.
    pub fn identity(&self)->&str { &self.identity }
    /// Immutable exact configuration bytes supplied to the factory.
    pub fn configuration(&self)->&[u8] { &self.configuration }
    /// Explicit seed; use is the selected implementation's contract.
    pub fn seed(&self)->u64 { self.seed }
    /// Number of admitted evaluation ticks.
    pub fn steps(&self)->usize { self.steps.len() }
    /// Conservative retained case accounting, not exact process RSS.
    pub fn accounted_bytes(&self)->usize { self.accounted }
}
fn add(a:usize,b:usize)->Result<usize,ReplayError> { a.checked_add(b).ok_or(ReplayError::Limit) }

/// Trusted compiled program selection. Each call must construct fresh isolated
/// state, not share mutable state with previous runs or live control.
pub trait ReplayProgram {
    /// Actual compiled implementation invoked by the runner.
    type Instance: Processor<Input=ReplayInput,Output=Vec<u8>>;
    /// Caller-declared label checked against the case before initialization.
    fn identity(&self)->&str;
    /// Construct from the case's immutable configuration and seed. Invalid input
    /// representation/configuration must return an explicit ComponentError.
    fn create(&self,configuration:&[u8],seed:u64)->Result<Self::Instance,ComponentError>;
}

/// Terminal outcome. Only Matched certifies complete exact comparison.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum ReplayOutcome {
    /// Every admitted step ran and all output batches matched exactly.
    Matched,
    /// Execution completed with output differences.
    Mismatched,
    /// Declared case/program labels did not match; nothing initialized.
    IdentityMismatch,
    /// Factory returned an error; no processor invocation occurred.
    InitializationFailed,
    /// Processor returned an error; comparison is incomplete.
    ProcessorFailed,
    /// Returned output count or payload exceeded policy; comparison is incomplete.
    OutputLimit,
}
/// Positional difference within a tick. Reordering is reported as changed slots.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum ReplayDifference {
    /// Both values exist but their bytes differ.
    Changed,
    /// Expected value has no corresponding actual value.
    Missing,
    /// Actual value has no corresponding expected value.
    Extra,
}
/// Fixed-size diagnostic; never retains payloads or arbitrary error strings.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct ReplayMismatch {
    /// Evaluation index in the admitted case.
    pub step:usize,
    /// Position within that tick's output batch.
    pub output:usize,
    /// Difference classification.
    pub difference:ReplayDifference,
    /// Expected byte length, if a value exists.
    pub expected_bytes:Option<usize>,
    /// Actual byte length, if a value exists.
    pub actual_bytes:Option<usize>,
}
/// Bounded evidence borrowing the exact case; no output payloads retained.
#[derive(Debug)]
pub struct ReplayReport<'a> {
    /// Exact immutable case used for this run.
    pub case:&'a ReplayCase,
    /// Rust implementation type selected by the factory; diagnostic, not attestation.
    pub implementation:&'static str,
    /// Terminal status; failure cannot be reported as a match.
    pub outcome:ReplayOutcome,
    /// Whether the selected factory returned a fresh instance successfully.
    pub initialized:bool,
    /// Number of process calls entered, including a call that returned an error.
    pub invoked:usize,
    /// Number of successfully returned, admitted output batches.
    pub completed:usize,
    /// Total admitted actual output count.
    pub outputs:usize,
    /// All positional differences, including those omitted from diagnostics.
    pub differences:usize,
    /// Bounded diagnostic prefix, in tick/output order.
    pub diagnostics:Vec<ReplayMismatch>,
}
impl ReplayReport<'_> {
    /// Number of differences omitted after the diagnostic cap.
    pub fn omitted(&self)->usize { self.differences-self.diagnostics.len() }
}

/// Execute a fresh selected processor under a private replay clock and compare
/// exact bytes per tick. No sorting, tolerance, source-time scheduling or output
/// resynchronization occurs. Expected output timing is the batch's evaluation tick.
/// Panics/hangs are not converted to successful reports; supervise externally.
pub fn run_replay<'a,P:ReplayProgram>(program:&P,case:&'a ReplayCase)->ReplayReport<'a> {
    let mut report=ReplayReport {case,implementation:std::any::type_name::<P::Instance>(),outcome:ReplayOutcome::IdentityMismatch,initialized:false,invoked:0,completed:0,outputs:0,differences:0,diagnostics:Vec::with_capacity(case.limits.diagnostics)};
    debug_assert!(report.diagnostics.capacity()*std::mem::size_of::<ReplayMismatch>()<=case.limits.report_bytes);
    if program.identity()!=case.identity { return report; }
    let Ok(mut processor)=program.create(&case.configuration,case.seed) else { report.outcome=ReplayOutcome::InitializationFailed; return report; };
    report.initialized=true;
    let clock=ManualClock::new(case.start);
    for (index,step) in case.steps.iter().enumerate() {
        // All schedule/domain/arithmetic checks happened before admission.
        clock.set(step.evaluation).expect("admitted replay clock domain");
        report.invoked+=1;
        let ctx=TickContext {now:clock.now(),sequence:index as u64};
        let Ok(actual)=processor.process(&ctx,step.input.clone()) else { report.outcome=ReplayOutcome::ProcessorFailed; return report; };
        if actual.len()>case.limits.outputs-report.outputs || actual.iter().any(|v|v.len()>case.limits.payload) {
            report.outcome=ReplayOutcome::OutputLimit; return report;
        }
        report.outputs+=actual.len(); report.completed+=1;
        for output in 0..step.expected.len().max(actual.len()) {
            let expected=step.expected.get(output); let got=actual.get(output);
            if expected==got { continue; }
            report.differences+=1;
            if report.diagnostics.len()<case.limits.diagnostics {
                report.diagnostics.push(ReplayMismatch {step:index,output,difference:match (expected,got) {(None,_)=>ReplayDifference::Extra,(_,None)=>ReplayDifference::Missing,_=>ReplayDifference::Changed},expected_bytes:expected.map(Vec::len),actual_bytes:got.map(Vec::len)});
            }
        }
    }
    report.outcome=if report.differences==0 {ReplayOutcome::Matched} else {ReplayOutcome::Mismatched};
    report
}
