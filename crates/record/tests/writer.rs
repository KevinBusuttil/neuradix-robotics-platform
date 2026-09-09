//! Every adversarial scenario has an external child deadline; CI adds an outer timeout.
use neuradix_record::*;
use std::{cell::RefCell, collections::BTreeMap, io::{self, Write}, process::{Command, Stdio}, rc::Rc, time::{Duration, Instant}};

fn header() -> McapHeader { McapHeader { profile:"test".into(),library:"writer-test".into() } }
fn schema(id:u16) -> McapSchema { McapSchema { id,name:"schema".into(),encoding:"opaque".into(),data:vec![0,255,7] } }
fn channel(id:u16) -> McapChannel { McapChannel { id,schema_id:1,topic:"topic".into(),message_encoding:"raw".into(),metadata:BTreeMap::from([("clock.source".into(),"device".into()),("clock.log".into(),"host".into())]) } }
fn metadata(name:&str) -> McapMetadata { McapMetadata { name:name.into(),entries:BTreeMap::from([("one".into(),"value".into()),("two".into(),"value".into())]) } }
fn message(id:u16) -> McapMessageRef<'static> { McapMessageRef { channel_id:id,sequence:u32::MAX,log_time:u64::MAX,publish_time:0,data:b"payload" } }
fn populate<W:Write>(w:&mut McapStreamWriter<W>) -> Result<()> {
    w.schema(&schema(1))?; w.schema(&schema(2))?;
    w.channel(&channel(0))?; w.channel(&channel(1))?;
    w.metadata(&metadata("a"))?; w.metadata(&metadata("b"))?;
    w.message(message(0))?; w.message(message(1))?; w.message(message(0))
}
fn output(policy:McapWriteLimits) -> Result<Vec<u8>> {
    let mut w=McapStreamWriter::new(Vec::new(),&header(),policy)?;
    populate(&mut w)?; w.finish()
}
fn frames(data:&[u8]) -> Vec<(u8,usize,usize)> {
    let mut p=8; let mut out=Vec::new();
    while p<data.len()-8 {
        let len=u64::from_le_bytes(data[p+1..p+9].try_into().unwrap()) as usize;
        out.push((data[p],p,len)); p+=9+len;
    }
    out
}
#[derive(Default)]
struct State { data:Vec<u8>, limit:usize, chunk:usize, flush_error:bool }
#[derive(Clone)]
struct Shared(Rc<RefCell<State>>);
impl Shared { fn new(limit:usize) -> Self { Self(Rc::new(RefCell::new(State { limit,chunk:3,..Default::default() }))) } }
impl Write for Shared {
    fn write(&mut self, data:&[u8]) -> io::Result<usize> {
        let mut s=self.0.borrow_mut();
        if s.data.len()>=s.limit { return Err(io::Error::other("injected write failure")); }
        let n=data.len().min(s.limit-s.data.len()).min(s.chunk);
        s.data.extend_from_slice(&data[..n]); Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> { if self.0.borrow().flush_error { Err(io::Error::other("injected flush failure")) } else { Ok(()) } }
}
fn run(case:&str) {
    let mut child=Command::new(std::env::current_exe().unwrap()).args(["--exact","writer_child","--nocapture"]).env("NEURADIX_WRITER_CASE",case).stdin(Stdio::null()).spawn().unwrap();
    let end=Instant::now()+Duration::from_secs(15);
    loop {
        if let Some(status)=child.try_wait().unwrap() { assert!(status.success(),"{case}"); break; }
        if Instant::now()>=end { child.kill().unwrap(); child.wait().unwrap(); panic!("writer deadline: {case}"); }
        std::thread::sleep(Duration::from_millis(5));
    }
}
macro_rules! case { ($name:ident) => { #[test] fn $name() { run(stringify!($name)); } }; }
case!(observable_streaming);
case!(exact_admission_boundaries);
case!(invalid_configuration);
case!(definitions_and_conflicts);
case!(io_failure_at_every_byte);
case!(flush_failure_and_abort);
case!(summary_crc_and_offsets);
case!(legacy_checked_migration);
case!(historical_summary_compatibility);
case!(bounded_state_across_messages);

#[test]
fn writer_child() {
    let Ok(case)=std::env::var("NEURADIX_WRITER_CASE") else { return; };
    let p=McapWriteLimits::default();
    match case.as_str() {
        "observable_streaming" => {
            let sink=Shared::new(usize::MAX);
            let mut w=McapStreamWriter::new(sink.clone(),&header(),p).unwrap();
            assert!(sink.0.borrow().data.starts_with(&MCAP_MAGIC));
            w.schema(&schema(1)).unwrap(); w.channel(&channel(0)).unwrap();
            let before=sink.0.borrow().data.len();
            w.message(message(0)).unwrap();
            assert_eq!(sink.0.borrow().data.len()-before,31+message(0).data.len());
            assert!(sink.0.borrow().data.ends_with(b"payload"));
            w.flush().unwrap();
            let expected=w.projected_final_bytes(); w.finish().unwrap();
            assert_eq!(sink.0.borrow().data.len() as u64,expected);
            McapArchive::from_reader(&sink.0.borrow().data[..],McapImportLimits::default()).unwrap();
        }
        "exact_admission_boundaries" => {
            let mut w=McapStreamWriter::new(Vec::new(),&header(),p).unwrap(); populate(&mut w).unwrap();
            let stats=w.stats().clone(); let bytes=w.finish().unwrap();
            let largest=frames(&bytes).iter().map(|f|f.2).max().unwrap();
            for policy in [p.with_output_bytes(bytes.len() as u64).unwrap(),p.with_record_bytes(largest).unwrap(),p.with_records(stats.records+3).unwrap(),p.with_messages(3).unwrap(),p.with_message_bytes(7).unwrap(),p.with_schemas(2).unwrap(),p.with_channels(2).unwrap(),p.with_metadata(2).unwrap(),p.with_entries(2).unwrap(),p.with_string_bytes(12).unwrap(),p.with_state_bytes(stats.state_bytes as usize).unwrap()] { assert_eq!(output(policy).unwrap(),bytes); }
            for policy in [p.with_output_bytes(bytes.len() as u64-1).unwrap(),p.with_record_bytes(largest-1).unwrap(),p.with_records(stats.records+2).unwrap(),p.with_messages(2).unwrap(),p.with_message_bytes(6).unwrap(),p.with_schemas(1).unwrap(),p.with_channels(1).unwrap(),p.with_metadata(1).unwrap(),p.with_entries(1).unwrap(),p.with_string_bytes(11).unwrap(),p.with_state_bytes(stats.state_bytes as usize-1).unwrap()] { assert!(output(policy).is_err()); }
            // Empty output and statistics growth also reserve exact tail bytes.
            let w=McapStreamWriter::new(Vec::new(),&header(),p).unwrap(); let size=w.projected_final_bytes();
            assert_eq!(w.finish().unwrap().len() as u64,size);
            McapStreamWriter::new(Vec::new(),&header(),p.with_output_bytes(size).unwrap()).unwrap().finish().unwrap();
            assert!(McapStreamWriter::new(Vec::new(),&header(),p.with_output_bytes(size-1).unwrap()).is_err());
            let mut w=McapStreamWriter::new(Vec::new(),&header(),p.with_record_bytes(55).unwrap()).unwrap();
            let mut c=channel(0); c.schema_id=0; c.metadata.clear(); w.channel(&c).unwrap();
            assert!(w.message(message(0)).is_err()); // Statistics would grow from 46 to 56.
        }
        "invalid_configuration" => {
            assert!(p.with_output_bytes(31).is_err()); assert!(p.with_output_bytes(u64::MAX).is_err());
            assert!(p.with_record_bytes(31).is_err()); assert!(p.with_record_bytes(usize::MAX).is_err());
            assert!(p.with_message_bytes(0).is_err()); assert!(p.with_message_bytes(usize::MAX).is_err());
            assert!(p.with_string_bytes(0).is_err()); assert!(p.with_string_bytes(usize::MAX).is_err());
            assert!(p.with_records(0).is_err()); assert!(p.with_records(u64::MAX).is_err());
            assert!(p.with_messages(0).is_err()); assert!(p.with_messages(u64::MAX).is_err());
            assert!(p.with_schemas(0).is_err()); assert!(p.with_schemas(65536).is_err());
            assert!(p.with_channels(0).is_err()); assert!(p.with_channels(65537).is_err());
            assert!(p.with_metadata(0).is_err()); assert!(p.with_metadata(65537).is_err());
            assert!(p.with_entries(0).is_err()); assert!(p.with_entries(65537).is_err());
            assert!(p.with_state_bytes(1023).is_err()); assert!(p.with_state_bytes(usize::MAX).is_err());
            for policy in [p.with_record_bytes(32).unwrap(),p.with_records(3).unwrap(),p.with_state_bytes(1024).unwrap()] { assert!(McapStreamWriter::new(Vec::new(),&header(),policy).is_err()); }
        }
        "definitions_and_conflicts" => {
            for kind in 0..6 {
                let sink=Shared::new(usize::MAX); let mut w=McapStreamWriter::new(sink.clone(),&header(),p).unwrap(); populate(&mut w).unwrap();
                let before=w.stats().clone(); w.schema(&schema(1)).unwrap(); w.channel(&channel(0)).unwrap(); w.metadata(&metadata("a")).unwrap(); assert_eq!(*w.stats(),before);
                let result=match kind {
                    0=>{ let mut s=schema(1); s.data.push(1); w.schema(&s) },
                    1=>{ let mut c=channel(0); c.topic.push('x'); w.channel(&c) },
                    2=>{ let mut m=metadata("a"); m.entries.clear(); w.metadata(&m) },
                    3=>w.schema(&schema(0)),
                    4=>{ let mut c=channel(9); c.schema_id=99; w.channel(&c) },
                    _=>w.message(message(99)),
                };
                assert!(result.is_err()); assert!(w.is_failed());
                let n=sink.0.borrow().data.len(); assert!(matches!(w.message(message(0)),Err(RecordError::McapWriterFailed))); assert!(w.finish().is_err()); assert_eq!(sink.0.borrow().data.len(),n);
            }
        }
        "io_failure_at_every_byte" => {
            let expected=output(p).unwrap();
            for limit in 0..expected.len() {
                let sink=Shared::new(limit);
                let result=(|| { let mut w=McapStreamWriter::new(sink.clone(),&header(),p)?; populate(&mut w)?; w.finish() })();
                assert!(result.is_err(),"accepted partial output at {limit}");
                assert_eq!(sink.0.borrow().data.len(),limit);
                assert_eq!(sink.0.borrow().data,expected[..limit]);
            }
            let sink=Shared::new(usize::MAX); sink.0.borrow_mut().chunk=0;
            assert!(McapStreamWriter::new(sink,&header(),p).is_err());
        }
        "flush_failure_and_abort" => {
            for action in 0..4 {
                let sink=Shared::new(usize::MAX); let mut w=McapStreamWriter::new(sink.clone(),&header(),p).unwrap(); populate(&mut w).unwrap();
                let before=sink.0.borrow().data.len();
                match action {
                    0=>{ drop(w); assert_eq!(sink.0.borrow().data.len(),before); },
                    1=>{ w.abort(); assert_eq!(sink.0.borrow().data.len(),before); },
                    2=>{ sink.0.borrow_mut().flush_error=true; assert!(w.flush().is_err()); assert!(w.is_failed()); assert!(w.finish().is_err()); assert_eq!(sink.0.borrow().data.len(),before); },
                    _=>{ sink.0.borrow_mut().flush_error=true; assert!(w.finish().is_err()); },
                }
            }
        }
        "summary_crc_and_offsets" => {
            let bytes=output(p).unwrap(); let f=frames(&bytes);
            assert_eq!(f.iter().map(|f|f.0).collect::<Vec<_>>(),vec![1,3,3,4,4,12,12,5,5,5,15,11,2]);
            let data_end=f[f.len()-3].1; let summary=f[f.len()-2].1; let footer=f[f.len()-1].1;
            assert_eq!(u32::from_le_bytes(bytes[data_end+9..data_end+13].try_into().unwrap()),crc32fast::hash(&bytes[..data_end]));
            assert_eq!(u64::from_le_bytes(bytes[footer+9..footer+17].try_into().unwrap()),summary as u64);
            assert_eq!(&bytes[footer+17..footer+25],&[0;8]);
            assert_eq!(u32::from_le_bytes(bytes[footer+25..footer+29].try_into().unwrap()),crc32fast::hash(&bytes[summary..footer+25]));
            let a=McapArchive::from_reader(&bytes[..],McapImportLimits::default()).unwrap();
            assert_eq!(a.summary().channel_message_counts(),&BTreeMap::from([(0,2),(1,1)]));
            assert_eq!(a.messages()[0].log_time,u64::MAX); assert_eq!(a.messages()[0].publish_time,0);
        }
        "legacy_checked_migration" => legacy(),
        "historical_summary_compatibility" => historical(),
        "bounded_state_across_messages" => {
            let mut w=McapStreamWriter::new(io::sink(),&header(),p.with_output_bytes(256<<20).unwrap()).unwrap(); w.schema(&schema(1)).unwrap(); w.channel(&channel(0)).unwrap();
            let state=w.stats().state_bytes; let payload=vec![0;64<<10];
            for sequence in 0..2048 { w.message(McapMessageRef { data:&payload,sequence,..message(0) }).unwrap(); assert_eq!(w.stats().state_bytes,state); }
            assert!(w.projected_final_bytes()>128<<20); w.finish().unwrap();
        }
        other=>panic!("unknown case {other}"),
    }
}
fn manifest() -> RecordingManifest {
    RecordingManifest::builder("legacy").channel(Channel { id:0,name:"channel".into(),schema_id:"identity".into(),clock_domain:"monotonic".into() }).build()
}
fn legacy() {
    use neuradix_time::{ClockDomain,Timestamp};
    for (id,sequence,domain,nanos) in [(9,0,ClockDomain::Monotonic,0),(0,0,ClockDomain::Simulation,0),(0,u64::MAX,ClockDomain::Monotonic,0),(0,0,ClockDomain::Monotonic,-1),(0,0,ClockDomain::Monotonic,i128::MAX)] {
        let mut w=McapWriter::new(Vec::new(),&manifest()).unwrap();
        assert!(w.write_record(id,sequence,Timestamp::new(domain,nanos),b"x").is_err()); assert!(w.is_failed()); assert!(w.finish().is_err());
    }
    let mut m=manifest(); m.channels[0].clock_domain="unknown".into(); assert!(McapWriter::new(Vec::new(),&m).is_err());
    let mut m=manifest(); m.channels.push(m.channels[0].clone()); assert!(McapWriter::new(Vec::new(),&m).is_err());
    let mut m=manifest(); m.note=Some("x".repeat(65536)); assert!(McapWriter::new(Vec::new(),&m).is_err());
}
fn historical() {
    // Reconstruct exactly the documented old interleaved-summary/zero-CRC shape
    // from valid definitions; this is compatibility, not independent interchange.
    let mut m=manifest(); let mut c=m.channels[0].clone(); c.id=1; m.channels.push(c);
    let bytes=McapWriter::new(Vec::new(),&m).unwrap().finish().unwrap();
    let f=frames(&bytes); let end=f.iter().find(|f|f.0==15).unwrap().1;
    let mut old=bytes[..end].to_vec(); old.push(15); old.extend_from_slice(&4u64.to_le_bytes()); old.extend_from_slice(&[0;4]);
    let summary=old.len() as u64;
    for (op,pos,len) in &f { if *op==3 || *op==4 { old.extend_from_slice(&bytes[*pos..pos+9+len]); } }
    let (_,pos,len)=f.iter().find(|f|f.0==11).unwrap(); old.extend_from_slice(&bytes[*pos..pos+9+len]);
    old.push(2); old.extend_from_slice(&20u64.to_le_bytes()); old.extend_from_slice(&summary.to_le_bytes()); old.extend_from_slice(&[0;12]); old.extend_from_slice(&MCAP_MAGIC);
    assert_eq!(McapRecording::from_bytes(&old).unwrap().manifest(),&m);
}
