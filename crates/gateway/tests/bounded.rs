//! Bounded behaviour under sustained traffic: after initialization the gateway
//! allocates nothing per chunk or frame, whether frames are delivered or
//! rejected, and its state does not grow.
//!
//! Allocations are counted on the current thread while pushing; the handler
//! only increments counters.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use neuradix_embedded_transport::ChannelBinding;
use neuradix_gateway::Handle;
use neuradix_gateway::reference::{
    self, SimulatedProducer, TINY_TELEMETRY_ID, TinyTelemetry, VEHICLE_DEPTH_ID, VehicleDepth,
};

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNTING.try_with(|on| {
            if on.get() {
                ALLOCATIONS.with(|n| n.set(n.get() + 1));
            }
        });
        // SAFETY: forwards the caller's layout contract to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr/layout come from the matching System allocation above.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn counted<R>(f: impl FnOnce() -> R) -> (R, usize) {
    ALLOCATIONS.with(|n| n.set(0));
    COUNTING.with(|on| on.set(true));
    let r = f();
    COUNTING.with(|on| on.set(false));
    (r, ALLOCATIONS.with(Cell::get))
}

#[derive(Default)]
struct Counter {
    depth: u64,
    tiny: u64,
}
impl Handle<VehicleDepth> for Counter {
    fn handle(&mut self, _: &ChannelBinding, _: VehicleDepth) {
        self.depth += 1;
    }
}
impl Handle<TinyTelemetry> for Counter {
    fn handle(&mut self, _: &ChannelBinding, _: TinyTelemetry) {
        self.tiny += 1;
    }
}

#[test]
fn b1_repeated_rejected_and_valid_traffic_allocates_nothing() {
    let mut p = SimulatedProducer::reference().unwrap();
    let tag = p.manifest_tag();
    let raw = SimulatedProducer::<{ reference::CHANNEL_CAPACITY }>::raw_envelope;
    let mut corrupt = p.vehicle_depth(
        VEHICLE_DEPTH_ID,
        &VehicleDepth {
            depth: 1.0,
            uncertainty: 0.0,
        },
    );
    let crc = corrupt.len() - 1;
    corrupt[crc] ^= 1;
    let mut bad_bool = [0; TinyTelemetry::WIRE_LEN];
    bad_bool[0] = 9;
    // One of each rejection, then one valid frame per route.
    let rejected: Vec<u8> = [
        corrupt,
        p.frame(&[0; reference::FRAME_CAPACITY + 1]),
        p.frame(&raw(9, tag, &[0; 16])),
        p.frame(&raw(0, tag, &[0; 16])),
        p.frame(&raw(1, tag, &[0; 15])),
        p.frame(&raw(1, [0, 0, 0, 0], &[0; 16])),
        p.frame(&[0; 16]),
        p.frame(&raw(1, tag, &[])[..7]),
        p.frame(&raw(2, tag, &bad_bool)),
        vec![0x00, 0xAA, 0x13, 0x55], // line noise
    ]
    .concat();
    let valid = [
        p.vehicle_depth(
            VEHICLE_DEPTH_ID,
            &VehicleDepth {
                depth: 2.0,
                uncertainty: 0.1,
            },
        ),
        p.tiny_telemetry(
            TINY_TELEMETRY_ID,
            &TinyTelemetry {
                a_enabled: true,
                b_measurement: 0.0,
                c_signed32: 0,
                d_signed64: 0,
                e_unsigned32: 0,
                z_unsigned64: 0,
            },
        ),
    ]
    .concat();

    let mut g = reference::gateway::<Counter>(reference::MANIFEST_JSON).unwrap();
    let mut h = Counter::default();
    let size = std::mem::size_of_val(&g);
    const ROUNDS: u64 = 5_000;
    let (_, allocations) = counted(|| {
        for _ in 0..ROUNDS {
            g.push(&rejected, &mut h);
            // Also in odd-sized fragments.
            for chunk in valid.chunks(7) {
                g.push(chunk, &mut h);
            }
        }
    });
    assert_eq!(allocations, 0, "allocations while routing");
    assert_eq!(std::mem::size_of_val(&g), size);
    let s = g.stats();
    assert_eq!((h.depth, h.tiny), (ROUNDS, ROUNDS));
    assert_eq!(s.delivered, 2 * ROUNDS);
    assert_eq!(s.corrupt, 2 * ROUNDS);
    assert_eq!(s.unknown_channel, 2 * ROUNDS);
    assert_eq!(
        (
            s.length_mismatch,
            s.manifest_mismatch,
            s.bad_magic,
            s.truncated,
            s.decode_rejected
        ),
        (ROUNDS, ROUNDS, ROUNDS, ROUNDS, ROUNDS)
    );
    assert_eq!(s.rejected(), 9 * ROUNDS);
    assert_eq!(s.bytes, ROUNDS * (rejected.len() + valid.len()) as u64);
}
