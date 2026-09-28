//! Shared fault-injecting reservation store for host tests (WP-A04.4).
//!
//! Included with `#[path]` by command-core, embedded-core and safety tests.
//! Fixed arrays only, no heap, and deliberately NOT `Clone` (contract C5).
//!
//! Model: two durable 64-byte slots, each with an optional marginal ("weak") tail
//! left by an interrupted program, a volatile write cache used only by the `Lie`
//! fault, a boot counter, a `dead` flag set by crash faults, a global operation
//! counter across boots, a plan of up to two faults keyed by operation index,
//! per-slot call counters and a 256-entry ring log.
#![allow(dead_code)]

use neuradix_command_core::reservation::{RECORD_BYTES, ReservationStore, Slot, StoreError};

/// Durable effect of an interrupted or failed write on the addressed slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tear {
    /// Slot keeps its old bytes.
    Unchanged,
    /// Slot erased.
    Erased,
    /// First `k` bytes new, the rest old.
    PrefixOverOld(usize),
    /// First `k` bytes new, the rest erased.
    PrefixOverErased(usize),
    /// Whole new record written.
    Complete,
    /// First `k` bytes new; bytes `k..` marginal (resolved per `WeakPolicy`).
    WeakTail(usize),
}

/// Injected fault at a given operation index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Power loss: on a write, apply the tear then die; on a read, die first.
    Crash(Tear),
    /// Write returns `Err(Io)` with the tear's durable effect; not dead.
    WriteErr(Tear),
    /// Write returns `Ok` but changes nothing.
    SilentDrop,
    /// Write returns `Ok` but lands only in a volatile cache lost at reboot.
    Lie,
    /// Read returns `Err(Io)`.
    ReadIo,
    /// Read returns `Err(Corrupt)`.
    ReadCorrupt,
    /// Read or write returns `Err(Unavailable)`.
    Unavailable,
}

/// How marginal bytes resolve on read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakPolicy {
    /// Always read as erased.
    AlwaysErased,
    /// Always read as the new bytes.
    AlwaysNew,
    /// New on even boots, erased on odd boots.
    AlternateEven,
    /// New on odd boots, erased on even boots.
    AlternateOdd,
    /// Resolved independently on every read (alternating per read).
    PerRead,
}

/// Media behaviour for marginal slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Media {
    /// Marginal bytes resolve per policy.
    Plain,
    /// Reading a marginal slot returns `Err(Corrupt)` (uncorrectable ECC).
    EccFlag,
    /// A marginal slot reads as erased and refuses writes (`Err(Io)`, no change).
    EccRefuse,
}

/// Operation kinds recorded in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// A read call.
    Read,
    /// A write call.
    Write,
}

/// One logged store call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Op {
    /// Global operation index (across boots).
    pub index: u64,
    /// Read or write.
    pub kind: OpKind,
    /// Addressed slot.
    pub slot: Slot,
    /// Whether an injected fault fired on this call.
    pub faulted: bool,
}

/// Snapshot of the durable image (for rollback characterization).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Image {
    durable: [[u8; RECORD_BYTES]; 2],
    weak: [Option<(usize, [u8; RECORD_BYTES])>; 2],
}

const LOG: usize = 256;

/// The fault-injecting store. NOT `Clone`.
#[derive(Debug)]
pub struct FaultStore {
    durable: [[u8; RECORD_BYTES]; 2],
    /// Marginal tail: bytes `k..` of the stored new record are weak.
    weak: [Option<(usize, [u8; RECORD_BYTES])>; 2],
    cache: [Option<[u8; RECORD_BYTES]>; 2],
    erased: u8,
    media: Media,
    policy: WeakPolicy,
    boot: u64,
    dead: bool,
    ops: u64,
    plan: [Option<(u64, Fault)>; 2],
    reads: [u64; 2],
    writes: [u64; 2],
    per_read: u64,
    log: [Option<Op>; LOG],
}

impl FaultStore {
    /// A store with both slots erased to `erased` (0xFF or 0x00).
    pub fn erased(erased: u8) -> Self {
        Self {
            durable: [[erased; RECORD_BYTES]; 2],
            weak: [None; 2],
            cache: [None; 2],
            erased,
            media: Media::Plain,
            policy: WeakPolicy::AlwaysErased,
            boot: 0,
            dead: false,
            ops: 0,
            plan: [None; 2],
            reads: [0; 2],
            writes: [0; 2],
            per_read: 0,
            log: [None; LOG],
        }
    }

    /// Select media behaviour and weak-bit policy.
    pub fn with_media(mut self, media: Media, policy: WeakPolicy) -> Self {
        self.media = media;
        self.policy = policy;
        self
    }

    /// Schedule `fault` at global operation index `op` (up to two entries).
    pub fn inject(&mut self, op: u64, fault: Fault) {
        let slot = self
            .plan
            .iter_mut()
            .find(|p| p.is_none())
            .expect("at most two planned faults");
        *slot = Some((op, fault));
    }

    /// Remove all planned faults.
    pub fn clear_faults(&mut self) {
        self.plan = [None; 2];
    }

    /// Whether a crash fault fired since the last reboot.
    pub fn dead(&self) -> bool {
        self.dead
    }

    /// Power cycle: clear `dead` and the volatile cache, advance the boot counter.
    pub fn reboot(&mut self) {
        self.dead = false;
        self.cache = [None; 2];
        self.boot += 1;
    }

    /// Boot counter.
    pub fn boot(&self) -> u64 {
        self.boot
    }

    /// Global operation counter (calls made so far, across boots).
    pub fn ops(&self) -> u64 {
        self.ops
    }

    /// Total calls `(reads, writes)` across both slots.
    pub fn calls(&self) -> (u64, u64) {
        (
            self.reads[0] + self.reads[1],
            self.writes[0] + self.writes[1],
        )
    }

    /// Per-slot `(reads, writes)`.
    pub fn slot_calls(&self, slot: Slot) -> (u64, u64) {
        (self.reads[slot.index()], self.writes[slot.index()])
    }

    /// Logged operations, oldest first (the last 256).
    pub fn log(&self) -> impl Iterator<Item = Op> + '_ {
        let start = self.ops.saturating_sub(LOG as u64);
        (start..self.ops).filter_map(move |i| self.log[(i % LOG as u64) as usize])
    }

    /// Order in which slots were written, oldest first, from the log.
    pub fn write_order(&self) -> impl Iterator<Item = Slot> + '_ {
        self.log()
            .filter(|op| op.kind == OpKind::Write)
            .map(|op| op.slot)
    }

    /// Current durable bytes of a slot (ignores cache and marginal resolution).
    pub fn raw(&self, slot: Slot) -> [u8; RECORD_BYTES] {
        self.durable[slot.index()]
    }

    /// Overwrite a slot's durable bytes directly (external writer / crafted record).
    pub fn set_raw(&mut self, slot: Slot, bytes: [u8; RECORD_BYTES]) {
        self.durable[slot.index()] = bytes;
        self.weak[slot.index()] = None;
    }

    /// Erase one slot directly.
    pub fn erase(&mut self, slot: Slot) {
        self.set_raw(slot, [self.erased; RECORD_BYTES]);
    }

    /// Flip bits of one durable byte (bit rot at rest).
    pub fn rot(&mut self, slot: Slot, byte: usize, mask: u8) {
        self.durable[slot.index()][byte] ^= mask;
    }

    /// Copy the durable image.
    pub fn snapshot(&self) -> Image {
        Image {
            durable: self.durable,
            weak: self.weak,
        }
    }

    /// Write back a durable image (rollback / restore / reflash).
    pub fn restore(&mut self, image: Image) {
        self.durable = image.durable;
        self.weak = image.weak;
        self.cache = [None; 2];
    }

    /// A second store with the same durable image (cloned device), fresh counters.
    pub fn clone_image(&self) -> Self {
        let mut other = Self::erased(self.erased).with_media(self.media, self.policy);
        other.durable = self.durable;
        other.weak = self.weak;
        other
    }

    fn next_fault(&mut self, index: u64) -> Option<Fault> {
        for entry in &mut self.plan {
            if let Some((op, fault)) = *entry
                && op == index
            {
                *entry = None;
                return Some(fault);
            }
        }
        None
    }

    fn record(&mut self, index: u64, kind: OpKind, slot: Slot, faulted: bool) {
        self.log[(index % LOG as u64) as usize] = Some(Op {
            index,
            kind,
            slot,
            faulted,
        });
    }

    fn weak_reads_new(&mut self) -> bool {
        match self.policy {
            WeakPolicy::AlwaysErased => false,
            WeakPolicy::AlwaysNew => true,
            WeakPolicy::AlternateEven => self.boot.is_multiple_of(2),
            WeakPolicy::AlternateOdd => !self.boot.is_multiple_of(2),
            WeakPolicy::PerRead => {
                self.per_read += 1;
                self.per_read.is_multiple_of(2)
            }
        }
    }

    fn resolve(&mut self, slot: Slot) -> Result<[u8; RECORD_BYTES], StoreError> {
        let i = slot.index();
        if let Some(cached) = self.cache[i] {
            return Ok(cached);
        }
        let mut bytes = self.durable[i];
        if let Some((k, new)) = self.weak[i] {
            match self.media {
                Media::EccFlag => return Err(StoreError::Corrupt),
                Media::EccRefuse => return Ok([self.erased; RECORD_BYTES]),
                Media::Plain => {
                    let fill_new = self.weak_reads_new();
                    for (b, byte) in bytes.iter_mut().enumerate().skip(k) {
                        *byte = if fill_new { new[b] } else { self.erased };
                    }
                }
            }
        }
        Ok(bytes)
    }

    fn apply(&mut self, slot: Slot, record: &[u8; RECORD_BYTES], tear: Tear) {
        let i = slot.index();
        let old = self.durable[i];
        let erased = [self.erased; RECORD_BYTES];
        self.weak[i] = None;
        self.cache[i] = None;
        let mut out = match tear {
            Tear::Unchanged => old,
            Tear::Erased => erased,
            Tear::Complete => *record,
            Tear::PrefixOverOld(_) => old,
            Tear::PrefixOverErased(_) | Tear::WeakTail(_) => erased,
        };
        if let Tear::PrefixOverOld(k) | Tear::PrefixOverErased(k) | Tear::WeakTail(k) = tear {
            let k = k.min(RECORD_BYTES);
            out[..k].copy_from_slice(&record[..k]);
            if let Tear::WeakTail(k) = tear
                && k < RECORD_BYTES
            {
                self.weak[i] = Some((k, *record));
            }
        }
        self.durable[i] = out;
    }
}

impl ReservationStore for FaultStore {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        let index = self.ops;
        self.ops += 1;
        self.reads[slot.index()] += 1;
        if self.dead {
            self.record(index, OpKind::Read, slot, false);
            return Err(StoreError::Unavailable);
        }
        let fault = self.next_fault(index);
        self.record(index, OpKind::Read, slot, fault.is_some());
        match fault {
            Some(Fault::Crash(_)) => {
                self.dead = true;
                return Err(StoreError::Unavailable);
            }
            Some(Fault::ReadIo) => return Err(StoreError::Io),
            Some(Fault::ReadCorrupt) => return Err(StoreError::Corrupt),
            Some(Fault::Unavailable) => return Err(StoreError::Unavailable),
            _ => {}
        }
        *buf = self.resolve(slot)?;
        Ok(())
    }

    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        let index = self.ops;
        self.ops += 1;
        self.writes[slot.index()] += 1;
        if self.dead {
            self.record(index, OpKind::Write, slot, false);
            return Err(StoreError::Unavailable);
        }
        let fault = self.next_fault(index);
        self.record(index, OpKind::Write, slot, fault.is_some());
        if self.media == Media::EccRefuse && self.weak[slot.index()].is_some() {
            return Err(StoreError::Io);
        }
        match fault {
            None => {
                self.apply(slot, record, Tear::Complete);
                Ok(())
            }
            Some(Fault::Crash(tear)) => {
                self.apply(slot, record, tear);
                self.dead = true;
                Err(StoreError::Unavailable)
            }
            Some(Fault::WriteErr(tear)) => {
                self.apply(slot, record, tear);
                Err(StoreError::Io)
            }
            Some(Fault::SilentDrop) => Ok(()),
            Some(Fault::Lie) => {
                self.cache[slot.index()] = Some(*record);
                Ok(())
            }
            Some(Fault::Unavailable) => Err(StoreError::Unavailable),
            Some(Fault::ReadIo | Fault::ReadCorrupt) => {
                self.apply(slot, record, Tear::Complete);
                Ok(())
            }
        }
    }
}

/// Provision `store` for `key` at `epoch` (trusted-test helper; requires the
/// `provisioning` feature, enabled for tests by dev-dependencies).
pub fn provisioned(
    store: &mut FaultStore,
    key: neuradix_command_core::reservation::ReservationKey,
    epoch: u64,
) {
    use neuradix_command_core::reservation::{NamespaceEpoch, ProvisionGuards, provision};
    provision(
        store,
        key,
        NamespaceEpoch::new(epoch).expect("nonzero epoch"),
        ProvisionGuards::default(),
    )
    .expect("provision");
}
