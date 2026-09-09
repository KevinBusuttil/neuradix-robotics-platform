//! Independent interoperability and adversarial import tests. Each case runs in
//! a child with an external wall deadline, including malformed-length attacks.
use neuradix_record::{
    MCAP_MAGIC, McapArchive, McapEventKind, McapImportLimits, RecordError, import_mcap,
};
use std::io::{Cursor, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn fixture(name: &str) -> Vec<u8> {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}.mcap.hex",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let text = text.trim();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn parts(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut pos = 8;
    let mut result = Vec::new();
    while pos < bytes.len() - 8 {
        let n = u64::from_le_bytes(bytes[pos + 1..pos + 9].try_into().unwrap()) as usize;
        result.push((bytes[pos], bytes[pos + 9..pos + 9 + n].to_vec()));
        pos += 9 + n;
    }
    result
}
fn append(out: &mut Vec<u8>, op: u8, data: &[u8]) {
    out.push(op);
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(data);
}
fn file(records: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut out = MCAP_MAGIC.to_vec();
    for (op, data) in records {
        append(&mut out, *op, data);
    }
    append(&mut out, 0x0f, &[0; 4]);
    append(&mut out, 2, &[0; 20]);
    out.extend_from_slice(&MCAP_MAGIC);
    out
}
fn data_records() -> Vec<(u8, Vec<u8>)> {
    parts(&fixture("uncompressed"))
        .into_iter()
        .take_while(|(op, _)| *op != 0x0f)
        .collect()
}
fn archive(bytes: &[u8], limits: McapImportLimits) -> neuradix_record::Result<McapArchive> {
    McapArchive::from_reader(bytes, limits)
}
fn reject(bytes: &[u8], limits: McapImportLimits) {
    assert!(archive(bytes, limits).is_err());
}
fn run(case: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "import_child", "--nocapture"])
        .env("NEURADIX_IMPORT_CASE", case)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{case}");
            break;
        }
        if Instant::now() >= end {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("import deadline: {case}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
macro_rules! case {
    ($name:ident) => {
        #[test]
        fn $name() {
            run(stringify!($name));
        }
    };
}
case!(independent_formats);
case!(unsupported_data);
case!(exact_limits);
case!(invalid_configuration);
case!(truncation_and_crc);
case!(references_and_conflicts);
case!(attacker_lengths);
case!(expansion_limits);
case!(streaming_backpressure);
case!(summary_integrity);

#[test]
fn import_child() {
    let Ok(case) = std::env::var("NEURADIX_IMPORT_CASE") else {
        return;
    };
    let limits = McapImportLimits::default();
    match case.as_str() {
        "independent_formats" => {
            let mut expected = None;
            for name in ["uncompressed", "chunked", "lz4"] {
                let bytes = fixture(name);
                let value = archive(&bytes, limits).unwrap();
                let s = value.summary();
                assert_eq!(s.header().profile, "independent-fixture");
                assert_eq!(s.header().library, "python mcap 1.3.1");
                assert_eq!(s.schemas().len(), 1);
                assert_eq!(s.channels().len(), 2);
                let schema = &s.schemas()[&1];
                assert_eq!(
                    (&*schema.name, &*schema.encoding, &*schema.data),
                    ("example/Bytes", "custom/schema", b"\0schema\xff".as_slice())
                );
                assert_eq!(s.channels()[&1].metadata["clock.source"], "device-boot");
                assert_eq!(s.channels()[&1].metadata["clock.log"], "host-boot");
                assert_eq!(s.channels()[&1].message_encoding, "opaque/custom");
                assert_eq!(s.channels()[&2].schema_id, 0);
                assert_eq!(s.metadata()["provenance"].entries["fixture"], "python-mcap");
                assert_eq!(s.metadata()["provenance"].entries["time"], "raw ns");
                let m = value.messages();
                assert_eq!(m.len(), 3);
                assert_eq!(
                    (
                        m[0].channel_id,
                        m[0].sequence,
                        m[0].log_time,
                        m[0].publish_time,
                        &*m[0].data
                    ),
                    (1, 0, 1, 0, b"\0\x01\xff".as_slice())
                );
                assert_eq!(
                    (
                        m[1].channel_id,
                        m[1].sequence,
                        m[1].log_time,
                        m[1].publish_time,
                        &*m[1].data
                    ),
                    (2, 0, 9, 7, b"".as_slice())
                );
                assert_eq!(
                    (m[2].sequence, m[2].log_time, m[2].publish_time, &*m[2].data),
                    (u32::MAX, u64::MAX, u64::MAX - 1, b"hello".as_slice())
                );
                if let Some(ref expected) = expected {
                    assert_eq!(m, expected);
                } else {
                    expected = Some(m.to_vec());
                }
                assert_eq!(s.stats().input_bytes, bytes.len() as u64);
                assert!(matches!(
                    value.try_into_recording(),
                    Err(RecordError::UnsupportedMcap(_))
                ));
            }
        }
        "unsupported_data" => {
            for name in ["zstd-unsupported", "attachment-unsupported"] {
                assert!(matches!(
                    archive(&fixture(name), limits),
                    Err(RecordError::UnsupportedMcap(_) | RecordError::UnsupportedMcapRecord(_))
                ));
            }
            for op in [0, 9, 0x0a, 0x10, 0x80, 0xff] {
                let mut records = data_records();
                records.push((op, Vec::new()));
                assert!(
                    matches!(archive(&file(&records), limits), Err(RecordError::UnsupportedMcapRecord(code)) if code == op)
                );
            }
            let mut records = data_records();
            records[0].1.push(0);
            assert!(matches!(
                archive(&file(&records), limits),
                Err(RecordError::UnsupportedMcap(_))
            ));
        }
        "exact_limits" => {
            let bytes = fixture("uncompressed");
            let a = archive(&bytes, limits).unwrap();
            let records = a.summary().stats().records;
            let state = a.summary().stats().state_bytes as usize;
            let retained = a.retained_bytes() as usize;
            let largest = parts(&bytes).iter().map(|(_, b)| b.len()).max().unwrap();
            for policy in [
                limits.with_input_bytes(bytes.len() as u64).unwrap(),
                limits.with_records(records).unwrap(),
                limits.with_messages(3).unwrap(),
                limits.with_channels(2).unwrap(),
                limits.with_schemas(1).unwrap(),
                limits.with_message_bytes(5).unwrap(),
                limits.with_record_bytes(largest).unwrap(),
                limits.with_state_bytes(state).unwrap(),
                limits.with_retained_bytes(retained).unwrap(),
                limits.with_entries(2).unwrap(),
                limits.with_string_bytes(19).unwrap(),
            ] {
                archive(&bytes, policy).unwrap();
            }
            for policy in [
                limits.with_input_bytes(bytes.len() as u64 - 1).unwrap(),
                limits.with_records(records - 1).unwrap(),
                limits.with_messages(2).unwrap(),
                limits.with_channels(1).unwrap(),
                limits.with_message_bytes(4).unwrap(),
                limits.with_record_bytes(largest - 1).unwrap(),
                limits.with_state_bytes(state - 1).unwrap(),
                limits.with_retained_bytes(retained - 1).unwrap(),
                limits.with_entries(1).unwrap(),
                limits.with_string_bytes(18).unwrap(),
            ] {
                reject(&bytes, policy);
            }
            let mut records = data_records();
            let mut schema = records.iter().find(|(op, _)| *op == 3).unwrap().1.clone();
            schema[..2].copy_from_slice(&2u16.to_le_bytes());
            records.push((3, schema));
            reject(&file(&records), limits.with_schemas(1).unwrap());
        }
        "invalid_configuration" => {
            assert!(limits.with_input_bytes(31).is_err());
            assert!(limits.with_input_bytes((1 << 40) + 1).is_err());
            assert!(limits.with_record_bytes(31).is_err());
            assert!(limits.with_record_bytes((64 << 20) + 1).is_err());
            assert!(limits.with_chunk_bytes(0).is_err());
            assert!(limits.with_chunk_bytes(usize::MAX).is_err());
            assert!(limits.with_decoded_bytes(0).is_err());
            assert!(limits.with_decoded_bytes(u64::MAX).is_err());
            assert!(limits.with_message_bytes(0).is_err());
            assert!(limits.with_message_bytes(usize::MAX).is_err());
            assert!(limits.with_string_bytes(0).is_err());
            assert!(limits.with_string_bytes(usize::MAX).is_err());
            assert!(limits.with_records(0).is_err());
            assert!(limits.with_records(u64::MAX).is_err());
            assert!(limits.with_messages(0).is_err());
            assert!(limits.with_messages(u64::MAX).is_err());
            assert!(limits.with_schemas(0).is_err());
            assert!(limits.with_schemas(65536).is_err());
            assert!(limits.with_channels(0).is_err());
            assert!(limits.with_channels(65537).is_err());
            assert!(limits.with_entries(0).is_err());
            assert!(limits.with_entries(65537).is_err());
            assert!(limits.with_state_bytes(1023).is_err());
            assert!(limits.with_state_bytes(usize::MAX).is_err());
            assert!(limits.with_retained_bytes(1023).is_err());
            assert!(limits.with_retained_bytes(usize::MAX).is_err());
        }
        "truncation_and_crc" => {
            for name in ["uncompressed", "chunked", "lz4"] {
                let bytes = fixture(name);
                for end in 0..bytes.len() {
                    reject(&bytes[..end], limits);
                }
                let mut corrupt = bytes.clone();
                let n = corrupt.len();
                corrupt[n - 9] ^= 1;
                reject(&corrupt, limits);
                let mut corrupt = bytes.clone();
                corrupt.push(0);
                reject(&corrupt, limits);
                // DataEnd CRC is independently generated and enabled.
                let mut pos = 8;
                while bytes[pos] != 0x0f {
                    pos += 9 + u64::from_le_bytes(bytes[pos + 1..pos + 9].try_into().unwrap())
                        as usize;
                }
                let mut corrupt = bytes.clone();
                corrupt[pos + 9] ^= 1;
                reject(&corrupt, limits);
            }
        }
        "references_and_conflicts" => {
            let original = data_records();
            for opcode in [3, 4] {
                let records: Vec<_> = original
                    .iter()
                    .filter(|(op, _)| *op != opcode)
                    .cloned()
                    .collect();
                reject(&file(&records), limits);
            }
            for opcode in [3, 4, 0x0c] {
                let index = original.iter().position(|(op, _)| *op == opcode).unwrap();
                let mut records = original.clone();
                records.push(records[index].clone());
                archive(&file(&records), limits).unwrap();
                let last = records.last_mut().unwrap().1.last_mut().unwrap();
                *last ^= 1;
                reject(&file(&records), limits);
            }
            let mut records = original.clone();
            let schema = records.iter_mut().find(|(op, _)| *op == 3).unwrap();
            schema.1[..2].fill(0);
            reject(&file(&records), limits);
        }
        "attacker_lengths" => {
            let mut bytes = MCAP_MAGIC.to_vec();
            bytes.push(1);
            bytes.extend_from_slice(&u64::MAX.to_le_bytes());
            reject(&bytes, limits);
            let mut records = data_records();
            records[0].1[..4].copy_from_slice(&u32::MAX.to_le_bytes());
            reject(&file(&records), limits);
            let mut records = data_records();
            let c = records.iter_mut().find(|(op, _)| *op == 4).unwrap();
            let mut pos = 4;
            for _ in 0..2 {
                let n = u32::from_le_bytes(c.1[pos..pos + 4].try_into().unwrap()) as usize;
                pos += 4 + n;
            }
            c.1[pos..pos + 4].copy_from_slice(&1u32.to_le_bytes());
            reject(&file(&records), limits);
            let mut records = data_records();
            let c = records.iter_mut().find(|(op, _)| *op == 4).unwrap();
            c.1[pos..pos + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            reject(&file(&records), limits);
        }
        "expansion_limits" => expansion(limits),
        "streaming_backpressure" => {
            let bytes = fixture("lz4");
            let mut input = Cursor::new(&bytes);
            let mut calls = 0;
            let result = import_mcap(&mut input, limits, |event| {
                calls += 1;
                if matches!(event.kind, McapEventKind::Message(_)) {
                    return Err(RecordError::UnsupportedMcap("caller stopped"));
                }
                Ok(())
            });
            assert!(result.is_err());
            assert!(calls > 1);
            assert!(input.position() < bytes.len() as u64);
            struct Slow<'a>(&'a [u8]);
            impl Read for Slow<'_> {
                fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                    let n = out.len().min(1);
                    self.0.read(&mut out[..n])
                }
            }
            assert_eq!(
                import_mcap(Slow(&bytes), limits, |_| Ok(()))
                    .unwrap()
                    .stats()
                    .messages,
                3
            );
            let mut seen = 0;
            let result = import_mcap(&bytes[..bytes.len() - 1], limits, |e| {
                if matches!(e.kind, McapEventKind::Message(_)) {
                    seen += 1;
                }
                Ok(())
            });
            assert_eq!(seen, 3);
            assert!(result.is_err());
            reject(&bytes, limits.with_retained_bytes(1024).unwrap());
            assert_eq!(
                import_mcap(
                    &bytes[..],
                    limits.with_retained_bytes(1024).unwrap(),
                    |_| Ok(())
                )
                .unwrap()
                .stats()
                .messages,
                3
            );
        }
        "summary_integrity" => {
            let bytes = file(&data_records());
            let mut records = parts(&bytes);
            records.last_mut().unwrap().1[..8].copy_from_slice(&u64::MAX.to_le_bytes());
            let mut out = MCAP_MAGIC.to_vec();
            for (op, data) in records {
                append(&mut out, op, &data);
            }
            out.extend_from_slice(&MCAP_MAGIC);
            reject(&out, limits);
            let mut records = data_records();
            records.push((1, records[0].1.clone()));
            reject(&file(&records), limits);
        }
        other => panic!("unknown case {other}"),
    }
}

fn expansion(limits: McapImportLimits) {
    let bytes = fixture("lz4");
    let a = archive(&bytes, limits).unwrap();
    let largest = a.summary().stats().largest_chunk_bytes;
    let total = a.summary().stats().decoded_bytes;
    archive(
        &bytes,
        limits
            .with_chunk_bytes(largest)
            .unwrap()
            .with_decoded_bytes(total)
            .unwrap(),
    )
    .unwrap();
    reject(&bytes, limits.with_chunk_bytes(largest - 1).unwrap());
    reject(&bytes, limits.with_decoded_bytes(total - 1).unwrap());
    for size in [0, 1, u64::MAX] {
        let mut records = parts(&bytes);
        let c = records.iter_mut().find(|(op, _)| *op == 6).unwrap();
        c.1[16..24].copy_from_slice(&size.to_le_bytes());
        let mut out = MCAP_MAGIC.to_vec();
        for (op, data) in records {
            append(&mut out, op, &data);
        }
        out.extend_from_slice(&MCAP_MAGIC);
        reject(&out, limits);
    }
    // An LZ4 frame without a content-size field must still stop at the decoded cap.
    use std::io::Write;
    let mut encoder = lz4::EncoderBuilder::new().build(Vec::new()).unwrap();
    encoder.write_all(&vec![0u8; 1 << 20]).unwrap();
    let (compressed, result) = encoder.finish();
    result.unwrap();
    let mut body = vec![0u8; 28];
    body[16..24].copy_from_slice(&32u64.to_le_bytes());
    body.extend_from_slice(&3u32.to_le_bytes());
    body.extend_from_slice(b"lz4");
    body.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    body.extend_from_slice(&compressed);
    let records = vec![data_records()[0].clone(), (6, body)];
    reject(&file(&records), limits);
}
