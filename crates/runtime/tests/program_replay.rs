//! Adversarial replay cases run in externally timed children, never CI-hanging loops.
use neuradix_runtime::{Processor,TickContext,ComponentError,run_lockstep};
use neuradix_runtime::replay::*;
use neuradix_time::{ClockDomain,Timestamp,ManualClock};
use std::{cell::Cell,rc::Rc,process::{Command,Stdio},time::{Duration,Instant}};

struct Program { mode:u8,created:Rc<Cell<usize>>,invoked:Rc<Cell<usize>> }
struct Instance { mode:u8,total:u8,gain:u8,calls:Rc<Cell<usize>> }
impl ReplayProgram for Program {
    type Instance=Instance;
    fn identity(&self)->&str { "test" }
    fn create(&self,config:&[u8],seed:u64)->Result<Instance,ComponentError> {
        self.created.set(self.created.get()+1);
        if config.len()!=1 || seed>255 { return Err(ComponentError::Failed("config".into())); }
        Ok(Instance {mode:self.mode,total:seed as u8,gain:config[0],calls:self.invoked.clone()})
    }
}
impl Processor for Instance {
    type Input=ReplayInput; type Output=Vec<u8>;
    fn process(&mut self,ctx:&TickContext,input:ReplayInput)->Result<Vec<Vec<u8>>,ComponentError> {
        self.calls.set(self.calls.get()+1);
        if input.data.len()!=1 { return Err(ComponentError::Failed("invalid representation".into())); }
        self.total=self.total.wrapping_add(input.data[0].wrapping_mul(self.gain));
        match self.mode {
            1=>Ok(vec![vec![self.total.wrapping_add(1)]]),
            2=>Ok(vec![]),
            3=>Ok(vec![vec![self.total],vec![9]]),
            4=>Err(ComponentError::Failed("processor failed".into())),
            5=>{ assert_eq!(ctx.now.domain(),ClockDomain::Replay); assert_eq!(ctx.now.as_nanos(),ctx.sequence as i128); assert_eq!(input.source_time.unwrap().domain(),ClockDomain::Sensor); Ok(vec![vec![self.total]]) },
            6=>Ok(vec![vec![1;17]]),
            7=>Ok(vec![vec![2],vec![1]]),
            _=>Ok(vec![vec![self.total]]),
        }
    }
}
fn program(mode:u8)->Program { Program {mode,created:Rc::new(Cell::new(0)),invoked:Rc::new(Cell::new(0))} }
fn time(n:i128)->Timestamp { Timestamp::new(ClockDomain::Replay,n) }
fn input(n:u8)->ReplayInput { ReplayInput {data:vec![n],source_time:Some(Timestamp::new(ClockDomain::Sensor,i128::MAX))} }
fn case(policy:ReplayLimits)->ReplayCase { ReplayCase::new("test",&[1],0,time(0),policy).unwrap() }
fn run(name:&str) {
    let mut c=Command::new(std::env::current_exe().unwrap()).args(["--exact","replay_child","--nocapture"]).env("NEURADIX_REPLAY_CASE",name).stdin(Stdio::null()).spawn().unwrap();
    let end=Instant::now()+Duration::from_secs(10);
    loop { if let Some(status)=c.try_wait().unwrap() {assert!(status.success(),"{name}");break;} if Instant::now()>=end {c.kill().unwrap();c.wait().unwrap();panic!("deadline {name}");} std::thread::sleep(Duration::from_millis(5)); }
}
macro_rules! test {($name:ident)=>{#[test] fn $name(){run(stringify!($name));}};}
test!(fresh_execution_and_changes);
test!(schedule_source_and_order);
test!(exact_storage_limits);
test!(bounded_diagnostics);
test!(explicit_failure_and_local_control);
test!(missing_extra_reordered);
test!(invalid_policy_and_clock_boundaries);

#[test]
fn replay_child() {
    let Ok(name)=std::env::var("NEURADIX_REPLAY_CASE") else{return;};
    let limits=ReplayLimits::default();
    match name.as_str() {
        "fresh_execution_and_changes"=>{
            let mut c=case(limits); c.push(time(0),&input(1),&[vec![1]]).unwrap(); c.push(time(1),&input(2),&[vec![3]]).unwrap();
            let p=program(0);
            for _ in 0..2 {let r=run_replay(&p,&c);assert_eq!(r.outcome,ReplayOutcome::Matched);assert_eq!((r.invoked,r.completed,r.outputs),(2,2,2));}
            assert_eq!(p.created.get(),2);assert_eq!(p.invoked.get(),4);
            assert_eq!(run_replay(&program(1),&c).outcome,ReplayOutcome::Mismatched);
            for (config,seed) in [([2],0),([1],1)] {let mut c=ReplayCase::new("test",&config,seed,time(0),limits).unwrap();c.push(time(0),&input(1),&[vec![1]]).unwrap();assert_eq!(run_replay(&p,&c).outcome,ReplayOutcome::Mismatched);}
            let empty=case(limits);let r=run_replay(&p,&empty);assert_eq!(r.invoked,0);assert!(r.initialized);assert_eq!(r.outcome,ReplayOutcome::Matched);
        }
        "schedule_source_and_order"=>{
            let mut c=case(limits);
            for n in 0..3 {c.push(time(n),&input(1),&[vec![n as u8+1]]).unwrap();}
            assert_eq!(run_replay(&program(5),&c).outcome,ReplayOutcome::Matched);
            let mut c=case(limits);c.push(time(0),&input(2),&[vec![2]]).unwrap();c.push(time(0),&input(1),&[vec![3]]).unwrap();assert_eq!(run_replay(&program(0),&c).outcome,ReplayOutcome::Matched);
            let mut reordered=case(limits);reordered.push(time(0),&input(1),&[vec![2]]).unwrap();reordered.push(time(0),&input(2),&[vec![3]]).unwrap();assert_eq!(run_replay(&program(0),&reordered).differences,1);
        }
        "exact_storage_limits"=>{
            let mut c=case(limits);for i in 0..3 {c.push(time(i),&input(1),&[vec![i as u8+1]]).unwrap();}
            let bytes=c.accounted_bytes();
            let p=ReplayLimits::new(3,3,1,bytes,0,0).unwrap();let mut c=case(p);
            for i in 0..3 {c.push(time(i),&input(1),&[vec![i as u8+1]]).unwrap();}
            assert_eq!(c.accounted_bytes(),bytes);assert!(c.push(time(3),&input(1),&[]).is_err());assert_eq!(c.steps(),3);assert_eq!(run_replay(&program(0),&c).outcome,ReplayOutcome::Matched);
            let mut c=case(ReplayLimits::new(3,3,1,bytes-1,0,0).unwrap());for i in 0..2 {c.push(time(i),&input(1),&[]).unwrap();} // atomic rejection independently measured below
            assert!(c.push(time(2),&ReplayInput {data:vec![1,2],source_time:None},&[]).is_err());assert_eq!(c.steps(),2);
            let mut c=case(ReplayLimits::new(3,1,16,4096,0,0).unwrap());c.push(time(0),&input(1),&[vec![1]]).unwrap();assert!(c.push(time(1),&input(1),&[vec![2]]).is_err());assert_eq!(run_replay(&program(3),&c).outcome,ReplayOutcome::OutputLimit);
            let mut c=case(ReplayLimits::new(3,3,16,4096,0,0).unwrap());c.push(time(0),&input(1),&[]).unwrap();assert_eq!(run_replay(&program(6),&c).outcome,ReplayOutcome::OutputLimit);
        }
        "bounded_diagnostics"=>{
            let p=ReplayLimits::new(1000,1000,16,1<<20,2,2*std::mem::size_of::<ReplayMismatch>()).unwrap();let mut c=case(p);
            for n in 0..1000 {c.push(time(n),&input(0),&[vec![1]]).unwrap();}
            let r=run_replay(&program(0),&c);assert_eq!(r.differences,1000);assert_eq!(r.diagnostics.len(),2);assert_eq!(r.omitted(),998);assert_eq!(r.diagnostics[1].step,1);
        }
        "explicit_failure_and_local_control"=>{
            let p=program(0);let c=ReplayCase::new("other",&[1],0,time(0),limits).unwrap();assert_eq!(run_replay(&p,&c).outcome,ReplayOutcome::IdentityMismatch);assert_eq!(p.created.get(),0);
            let c=ReplayCase::new("test",&[],0,time(0),limits).unwrap();let r=run_replay(&p,&c);assert_eq!(r.outcome,ReplayOutcome::InitializationFailed);assert_eq!(r.invoked,0);
            let mut c=case(limits);c.push(time(0),&input(1),&[vec![1]]).unwrap();let r=run_replay(&program(4),&c);assert_eq!(r.outcome,ReplayOutcome::ProcessorFailed);assert_eq!((r.invoked,r.completed),(1,0));
            let mut bad=case(limits);bad.push(time(0),&ReplayInput {data:vec![],source_time:None},&[]).unwrap();assert_eq!(run_replay(&p,&bad).outcome,ReplayOutcome::ProcessorFailed);
            // A failed replay instance does not mutate this independent conventional path.
            let mut local=p.create(&[1],0).unwrap();let clock=ManualClock::new(time(0));assert_eq!(run_lockstep(&clock,&mut local,[(time(0),input(2))]).unwrap(),vec![vec![2]]);
        }
        "missing_extra_reordered"=>{
            let mut c=case(limits);c.push(time(0),&input(1),&[vec![1]]).unwrap();assert_eq!(run_replay(&program(2),&c).diagnostics[0].difference,ReplayDifference::Missing);assert_eq!(run_replay(&program(3),&c).diagnostics[0].difference,ReplayDifference::Extra);
            let mut c=case(limits);c.push(time(0),&input(1),&[vec![1],vec![2]]).unwrap();let r=run_replay(&program(7),&c);assert_eq!(r.differences,2);assert!(r.diagnostics.iter().all(|d|d.difference==ReplayDifference::Changed));
        }
        "invalid_policy_and_clock_boundaries"=>{
            for args in [(0,1,1,1024,0,0),(1,0,1,1024,0,0),(1,1,0,1024,0,0),(1,1,1,1023,0,0),(1,1,1,1024,1,0),(usize::MAX,1,1,1024,0,0),(1,1,usize::MAX,1024,0,0),(1,1,1,usize::MAX,0,0),(1,1,1,1024,usize::MAX,usize::MAX)] {assert!(ReplayLimits::new(args.0,args.1,args.2,args.3,args.4,args.5).is_err());}
            assert!(ReplayCase::new("",&[],0,time(0),limits).is_err());assert!(ReplayCase::new(&"x".repeat(257),&[],0,time(0),limits).is_err());
            let mut c=case(limits);assert_eq!(c.push(Timestamp::new(ClockDomain::Utc,0),&input(0),&[]),Err(ReplayError::ClockMismatch));assert_eq!(c.push(time(-1),&input(0),&[]),Err(ReplayError::ClockRegression));assert_eq!(c.steps(),0);
            let mut c=ReplayCase::new("test",&[1],0,time(i128::MIN),limits).unwrap();assert_eq!(c.push(time(i128::MAX),&input(0),&[]),Err(ReplayError::ClockOverflow));c.push(time(-1),&input(0),&[vec![0]]).unwrap();assert_eq!(run_replay(&program(0),&c).outcome,ReplayOutcome::Matched);
            let mut c=ReplayCase::new("test",&[1],0,time(i128::MAX),limits).unwrap();c.push(time(i128::MAX),&input(0),&[vec![0]]).unwrap();assert_eq!(run_replay(&program(0),&c).outcome,ReplayOutcome::Matched);
        }
        _=>panic!("unknown scenario"),
    }
}
