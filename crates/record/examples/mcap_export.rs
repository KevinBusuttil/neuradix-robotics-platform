//! Independent-export and bounded-memory workload producer; no whole-file buffer.
use neuradix_record::{McapChannel, McapHeader, McapMessageRef, McapMetadata, McapSchema, McapStreamWriter, McapWriteLimits};
use std::{collections::BTreeMap, fs::File};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args:Vec<_>=std::env::args().collect();
    let mode=&args[1];
    let policy=McapWriteLimits::default().with_output_bytes(256<<20)?;
    let mut writer=McapStreamWriter::new(File::create(&args[2])?, &McapHeader { profile:"neuradix-export-evidence".into(), library:"neuradix-record/0.0.1".into() },policy)?;
    writer.schema(&McapSchema { id:42,name:"example/Bytes".into(),encoding:"custom/schema".into(),data:b"\0schema\xff".to_vec() })?;
    writer.channel(&McapChannel { id:7,schema_id:42,topic:"sensor/bytes".into(),message_encoding:"opaque/custom".into(),metadata:BTreeMap::from([("clock.source".into(),"device-boot".into()),("clock.log".into(),"host-boot".into())]) })?;
    writer.channel(&McapChannel { id:0,schema_id:0,topic:"raw".into(),message_encoding:String::new(),metadata:BTreeMap::from([("note".into(),"no inferred epoch".into())]) })?;
    writer.metadata(&McapMetadata { name:"provenance".into(),entries:BTreeMap::from([("producer".into(),"neuradix".into()),("time".into(),"raw ns".into())]) })?;
    if mode=="fixture" {
        for (channel_id,sequence,log_time,publish_time,data) in [(7,0,1,0,b"\0\x01\xff".as_slice()),(0,5,9,7,b"".as_slice()),(7,u32::MAX,u64::MAX,u64::MAX-1,b"hello".as_slice())] {
            writer.message(McapMessageRef { channel_id,sequence,log_time,publish_time,data })?;
        }
    } else {
        let mib:usize=mode.parse()?;
        let payload=vec![0xa5;64<<10];
        for index in 0..mib*16 {
            writer.message(McapMessageRef { channel_id:7,sequence:index as u32,log_time:index as u64,publish_time:index as u64,data:&payload })?;
        }
    }
    writer.flush()?;
    assert!(std::fs::metadata(&args[2])?.len()>8);
    println!("messages={} state_bytes={} projected_file_bytes={}",writer.stats().messages,writer.stats().state_bytes,writer.projected_final_bytes());
    writer.finish()?;
    Ok(())
}
