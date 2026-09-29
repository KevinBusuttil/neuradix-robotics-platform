//! Reference gateway, host simulation.
//!
//! generated producer payload → compact envelope → CRC frame → incremental
//! frame decoder → verified channel resolution → generated decoder → typed
//! handler, over two routes, followed by rejected traffic.
//!
//! ```sh
//! cargo run -p neuradix-gateway --example reference_gateway [-- <manifest.json>]
//! ```
//!
//! With no argument the checked-in configured manifest is used. A manifest that
//! does not match the expected routes and the compiled board table is refused
//! before any traffic (exit 2). The run checks its own results and exits 1 on
//! any unexpected outcome.

use std::io::Read as _;
use std::process::ExitCode;

use neuradix_embedded_transport::ChannelBinding;
use neuradix_gateway::reference::{
    self, SimulatedProducer, TINY_TELEMETRY_ID, TinyTelemetry, VEHICLE_DEPTH_ID, VehicleDepth,
};
use neuradix_gateway::{FrameOutcome, Handle, MAX_MANIFEST_BYTES, Rejection, Stats};

/// Typed handlers: print and count each value.
#[derive(Default)]
struct Console {
    depth: u32,
    tiny: u32,
}

impl Handle<VehicleDepth> for Console {
    fn handle(&mut self, channel: &ChannelBinding, v: VehicleDepth) {
        self.depth += 1;
        println!(
            "    handler  {} (ch {}): {v:?}",
            channel.name, channel.compact_id
        );
    }
}

impl Handle<TinyTelemetry> for Console {
    fn handle(&mut self, channel: &ChannelBinding, v: TinyTelemetry) {
        self.tiny += 1;
        println!(
            "    handler  {} (ch {}): {v:?}",
            channel.name, channel.compact_id
        );
    }
}

fn describe(outcome: FrameOutcome) {
    match outcome {
        FrameOutcome::Delivered {
            seq,
            sequence,
            channel,
        } => println!(
            "    frame    seq {seq} delivered on ch {} ({sequence:?})",
            channel.compact_id
        ),
        FrameOutcome::Rejected { seq, reason } => {
            let seq = seq.map_or("-".to_owned(), |s| s.to_string());
            let why = match reason {
                Rejection::CorruptFrame => "corrupt frame (CRC failure or oversized)".to_owned(),
                Rejection::Envelope(e) => format!("envelope refused: {e:?}"),
                Rejection::Decode { compact_id } => {
                    format!("generated decoder refused the body on ch {compact_id}")
                }
            };
            println!("    frame    seq {seq} rejected, no handler: {why}");
        }
    }
}

fn load_manifest() -> Result<String, String> {
    let Some(path) = std::env::args_os().nth(1) else {
        return Ok(reference::MANIFEST_JSON.to_owned());
    };
    let mut text = String::new();
    std::fs::File::open(&path)
        .and_then(|f| {
            f.take(MAX_MANIFEST_BYTES as u64 + 1)
                .read_to_string(&mut text)
        })
        .map_err(|e| format!("cannot read {}: {e}", path.to_string_lossy()))?;
    Ok(text)
}

fn main() -> ExitCode {
    println!("Neuradix reference gateway (host simulation: no board, no link session)");
    let manifest = match load_manifest() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    let mut gateway = match reference::gateway::<Console>(&manifest) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: gateway not started, no traffic processed: {e}");
            return ExitCode::from(2);
        }
    };
    let mut producer = SimulatedProducer::reference().expect("generated board table verifies");
    let mut console = Console::default();

    println!("\nidentity");
    println!("{}", gateway.identity(reference::simulated_claims()));
    println!("\nroutes (verified against the manifest, contracts, decoders and board table)");
    for (b, ty) in gateway.routes().routes() {
        println!(
            "  ch {} {:<15} -> {ty:<14} wire {} ({} B)",
            b.compact_id, b.name, b.wire_id, b.wire_len
        );
    }
    println!(
        "limits: frame buffer {} B, largest envelope {} B, table capacity {}",
        gateway.frame_capacity(),
        gateway.routes().max_envelope_len(),
        reference::CHANNEL_CAPACITY
    );

    let depth = VehicleDepth {
        depth: 12.5,
        uncertainty: 0.25,
    };
    let tiny = TinyTelemetry {
        a_enabled: true,
        b_measurement: 1.5,
        c_signed32: -7,
        d_signed64: -1_000_000_000_000,
        e_unsigned32: 42,
        z_unsigned64: u64::MAX,
    };
    let tag = producer.manifest_tag();
    let raw = SimulatedProducer::<{ reference::CHANNEL_CAPACITY }>::raw_envelope;
    let mut bad_bool = [0u8; TinyTelemetry::WIRE_LEN];
    tiny.encode(&mut bad_bool);
    bad_bool[0] = 2;

    // (label, chunks, discard a partial frame afterwards). Frames are built in
    // transmission order, so sequence numbers increase down the list.
    let mut steps: Vec<(&str, Vec<Vec<u8>>, bool)> = Vec::new();
    let both = [
        producer.vehicle_depth(VEHICLE_DEPTH_ID, &depth),
        producer.tiny_telemetry(TINY_TELEMETRY_ID, &tiny),
    ]
    .concat();
    steps.push(("two routes, two frames in one chunk", vec![both], false));
    let noisy = [
        vec![0x00, 0x13, 0xAA, 0x37],
        producer.vehicle_depth(VEHICLE_DEPTH_ID, &depth),
    ]
    .concat();
    steps.push((
        "line noise, then one frame split into 3-byte fragments",
        noisy.chunks(3).map(<[u8]>::to_vec).collect(),
        false,
    ));
    let mut corrupt = producer.vehicle_depth(VEHICLE_DEPTH_ID, &depth);
    let crc_byte = corrupt.len() - 1;
    corrupt[crc_byte] ^= 0xff;
    steps.push(("CRC failure", vec![corrupt], false));
    let oversized = [
        producer.frame(&[0u8; 100]),
        producer.tiny_telemetry(TINY_TELEMETRY_ID, &tiny),
    ]
    .concat();
    steps.push((
        "oversized frame (100-byte payload, 64-byte buffer), then a valid frame",
        vec![oversized],
        false,
    ));
    let unknown = vec![
        producer.frame(&raw(9, tag, &[0; 16])),
        producer.frame(&raw(0, tag, &[0; 16])),
    ];
    steps.push((
        "unknown compact ID 9 and reserved ID 0, correct manifest tag",
        unknown,
        false,
    ));
    let short = producer.frame(&raw(VEHICLE_DEPTH_ID, tag, &[0; 15]));
    steps.push(("wrong length on ch 1 (15 bytes)", vec![short], false));
    let other_tag = [tag[0] ^ 1, tag[1], tag[2], tag[3]];
    let foreign = producer.frame(&raw(VEHICLE_DEPTH_ID, other_tag, &[0; 16]));
    steps.push((
        "producer provisioned from another manifest (different tag)",
        vec![foreign],
        false,
    ));
    let bare = producer.frame(&[0u8; 16]);
    steps.push(("unenveloped payload", vec![bare], false));
    let bad = producer.frame(&raw(TINY_TELEMETRY_ID, tag, &bad_bool));
    steps.push(("non-canonical bool on ch 2", vec![bad], false));
    let partial = producer.vehicle_depth(VEHICLE_DEPTH_ID, &depth)[..10].to_vec();
    steps.push((
        "incomplete frame, discarded after an idle gap",
        vec![partial],
        true,
    ));
    let after = producer.tiny_telemetry(TINY_TELEMETRY_ID, &tiny);
    steps.push(("valid frame after the discard", vec![after], false));

    println!("\ntraffic");
    for (label, chunks, discard_after) in steps {
        println!("  {label}  [{} chunk(s)]", chunks.len());
        for chunk in chunks {
            gateway.push_observed(&chunk, &mut console, describe);
        }
        if discard_after {
            gateway.discard_partial();
            println!("    (partial frame discarded)");
        }
    }

    let s: Stats = *gateway.stats();
    println!("\nsummary");
    println!(
        "  bytes {}  frames {}  delivered {} (vehicle-depth {}, tiny-telemetry {})  rejected {}",
        s.bytes,
        s.frames,
        s.delivered,
        console.depth,
        console.tiny,
        s.rejected()
    );
    println!(
        "  corrupt {}  unknown channel {}  length {}  manifest tag {}  bad magic {}  decoder {}",
        s.corrupt,
        s.unknown_channel,
        s.length_mismatch,
        s.manifest_mismatch,
        s.bad_magic,
        s.decode_rejected
    );
    println!(
        "  sequence (observational, not enforced): missed {}  duplicates {}  reordered {}",
        s.missed, s.duplicates, s.reordered
    );

    let ok = s.delivered == u64::from(console.depth + console.tiny)
        && console.depth == 2
        && console.tiny == 3
        && s.corrupt == 2
        && s.unknown_channel == 2
        && s.length_mismatch == 1
        && s.manifest_mismatch == 1
        && s.bad_magic == 1
        && s.decode_rejected == 1;
    if ok {
        println!("\nresult: all outcomes as expected");
        ExitCode::SUCCESS
    } else {
        println!("\nresult: UNEXPECTED outcomes");
        ExitCode::from(1)
    }
}
