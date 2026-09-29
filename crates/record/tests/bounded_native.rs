//! Bounded native reads (`NativeRecording::from_reader`): limits apply while
//! reading, never after an unbounded allocation.
use std::io::Read;

use neuradix_record::*;
use neuradix_time::{ClockDomain, Timestamp};

fn recording(records: u64, payload: usize) -> Vec<u8> {
    let manifest = RecordingManifest::builder("bounded-native").build();
    let mut w = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    for i in 0..records {
        let at = Timestamp::new(ClockDomain::Simulation, -(i as i128));
        w.write_record(0, i, at, &vec![i as u8; payload]).unwrap();
    }
    w.finish().unwrap()
}

/// A reader that returns at most `step` bytes and an `Interrupted` error
/// before every read, counting the bytes it hands out.
struct Stingy<'a> {
    data: &'a [u8],
    step: usize,
    interrupt: bool,
    served: usize,
}
impl Read for Stingy<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let n = buf.len().min(self.step).min(self.data.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        self.served += n;
        Ok(n)
    }
}

#[test]
fn exact_limits_accept_and_one_less_rejects() {
    let bytes = recording(5, 10);
    let len = bytes.len() as u64;
    let ok =
        NativeRecording::from_reader(&bytes[..], NativeReadLimits::new(len, 5).unwrap()).unwrap();
    assert_eq!(
        ok.records(),
        NativeRecording::from_bytes(&bytes).unwrap().records()
    );
    let err = NativeRecording::from_reader(&bytes[..], NativeReadLimits::new(len - 1, 5).unwrap())
        .unwrap_err();
    assert!(
        matches!(err, RecordError::ReadLimit { kind: "input bytes", limit } if limit == len - 1)
    );
    let err = NativeRecording::from_reader(&bytes[..], NativeReadLimits::new(len, 4).unwrap())
        .unwrap_err();
    assert!(matches!(
        err,
        RecordError::ReadLimit {
            kind: "records",
            limit: 4
        }
    ));
}

#[test]
fn reading_stops_one_byte_past_the_limit() {
    // A 1 MiB stream against a 1 KiB cap: at most cap + 1 bytes are consumed.
    let bytes = recording(1, 1 << 20);
    let mut reader = Stingy {
        data: &bytes,
        step: 777,
        interrupt: false,
        served: 0,
    };
    let err = NativeRecording::from_reader(&mut reader, NativeReadLimits::new(1024, 10).unwrap())
        .unwrap_err();
    assert!(matches!(
        err,
        RecordError::ReadLimit {
            kind: "input bytes",
            ..
        }
    ));
    assert_eq!(reader.served, 1025);
}

#[test]
fn chunked_and_interrupted_reads_decode_identically() {
    let bytes = recording(300, 3);
    let reader = Stingy {
        data: &bytes,
        step: 5,
        interrupt: false,
        served: 0,
    };
    let got = NativeRecording::from_reader(reader, NativeReadLimits::default()).unwrap();
    let want = NativeRecording::from_bytes(&bytes).unwrap();
    assert_eq!(got.manifest(), want.manifest());
    assert_eq!(got.records(), want.records());
    // Signed timestamps survive.
    assert_eq!(
        got.records()[299].timestamp,
        Timestamp::new(ClockDomain::Simulation, -299)
    );
}

#[test]
fn invalid_limits_and_defaults() {
    assert!(matches!(
        NativeReadLimits::new(0, 1),
        Err(RecordError::InvalidReadLimit("max_bytes"))
    ));
    assert!(matches!(
        NativeReadLimits::new(1, 0),
        Err(RecordError::InvalidReadLimit("max_records"))
    ));
    let d = NativeReadLimits::default();
    assert_eq!(d.max_bytes(), 256 << 20);
    assert_eq!(d.max_records(), 1 << 20);
    // Truncated or foreign input still fails decoding, not the limits.
    let bytes = recording(2, 4);
    assert!(matches!(
        NativeRecording::from_reader(&bytes[..bytes.len() - 1], d),
        Err(RecordError::Truncated(_))
    ));
    assert!(matches!(
        NativeRecording::from_reader(&b"NOTREC\x01"[..], d),
        Err(RecordError::BadMagic)
    ));
}
