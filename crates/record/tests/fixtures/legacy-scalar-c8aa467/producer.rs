//! Historical fixture producer. Runs ONLY inside a worktree at
//! c8aa4671bcee8739354beb7880551db7f64314fa, as an example of that revision's
//! `neuradix-record` crate (see generate.sh). It uses that revision's generated
//! no_std encoder and native writer, and prints the recording as hex.
use std::path::Path;

use neuradix_contracts::{load_file, schema_identity};
use neuradix_record::{Channel, NativeRecordWriter, RecordingManifest, SoftwareId};
use neuradix_time::{ClockDomain, Timestamp};

mod generated {
    include!("legacy_probe_generated.rs");
}
mod depth {
    include!("vehicle_depth_generated.rs");
}
use depth::VehicleDepth;
use generated::LegacyProbe;

fn main() {
    let arg = |n: usize| std::env::args().nth(n).expect("contract paths");
    let schema = schema_identity(&load_file(Path::new(&arg(1))).expect("load probe"));
    assert_eq!(schema.as_str(), LegacyProbe::SCHEMA_ID);
    let depth_schema = schema_identity(&load_file(Path::new(&arg(2))).expect("load depth"));
    assert_eq!(depth_schema.as_str(), VehicleDepth::SCHEMA_ID);
    let manifest = RecordingManifest::builder("neuradix-record/legacy-fixture")
        .channel(Channel::new(
            1,
            "fixtures/legacy-probe",
            &schema,
            ClockDomain::Monotonic,
        ))
        .channel(Channel {
            id: 2,
            name: "fixtures/opaque-log".into(),
            schema_id: "opaque/application-octet-stream".into(),
            clock_domain: ClockDomain::Simulation.as_str().into(),
        })
        .channel(Channel::new(
            3,
            "navigation/vehicle-depth",
            &depth_schema,
            ClockDomain::Monotonic,
        ))
        .software(SoftwareId::new(
            "neuradix-embedded-codegen",
            "0.0.1+c8aa4671bcee8739354beb7880551db7f64314fa",
        ))
        .seed(7)
        .note("legacy scalar migration fixture")
        .build();
    let samples = [
        LegacyProbe {
            yaw: 1.5,
            pitch: -0.25,
            count: -7,
            armed: true,
            rate: 0.5,
            ticks: 1 << 40,
            offset: -(1 << 33),
            flags: 0xA5A5_0001,
        },
        LegacyProbe {
            yaw: -3.0,
            pitch: 2.75,
            count: 42,
            armed: false,
            rate: -8.0,
            ticks: 3,
            offset: 5,
            flags: 0,
        },
        LegacyProbe {
            yaw: 0.0,
            pitch: -0.0,
            count: i32::MIN,
            armed: true,
            rate: 1024.0,
            ticks: u64::MAX,
            offset: i64::MIN,
            flags: u32::MAX,
        },
    ];
    let mut writer = NativeRecordWriter::new(Vec::new(), &manifest).unwrap();
    let mut buf = [0u8; LegacyProbe::WIRE_LEN];
    let mono = |n: i128| Timestamp::new(ClockDomain::Monotonic, n);
    let sim = |n: i128| Timestamp::new(ClockDomain::Simulation, n);
    // Interleave an opaque channel; its bytes must never be reinterpreted.
    writer.write_record(2, 5, sim(-10), b"\x00\xffopaque").unwrap();
    for (i, sample) in samples.iter().enumerate() {
        assert_eq!(sample.encode(&mut buf), Some(LegacyProbe::WIRE_LEN));
        let at = 1_000_000_000 + i as i128 * 20_000_000;
        writer.write_record(1, 10 + i as u64 * 2, mono(at), &buf).unwrap();
        if i == 1 {
            writer.write_record(2, 9, sim(at), &[0x4E; 45]).unwrap();
        }
    }
    let mut depth_buf = [0u8; VehicleDepth::WIRE_LEN];
    for (i, (depth, uncertainty)) in [(12.5, 0.25), (-1.0, 3.0)].into_iter().enumerate() {
        let v = VehicleDepth { depth, uncertainty };
        assert_eq!(v.encode(&mut depth_buf), Some(VehicleDepth::WIRE_LEN));
        writer
            .write_record(3, i as u64, mono(2_000_000_000 + i as i128), &depth_buf)
            .unwrap();
    }
    let bytes = writer.finish().unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    for line in hex.as_bytes().chunks(64) {
        println!("{}", std::str::from_utf8(line).unwrap());
    }
}
