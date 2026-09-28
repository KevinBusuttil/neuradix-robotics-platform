//! Durable, allocation-free generation reservation for trusted startup.
//!
//! WP-A04.2 requires trusted initialization, on **every** startup, to reserve and
//! durably persist a generation greater than every value previously used for the
//! receiver/binding **before** activating it, covering reconstruction,
//! replacement, revocation/regrant and recovery. This module supplies that
//! allocator for one `(receiver, binding)` key over a two-slot store.
//!
//! Everything here is **trusted setup, board support or provisioning only**.
//! Components receive only actuator ports; no port, payload or report type can
//! produce, hold or consume a [`ReservedGeneration`], store or reserver.
//!
//! # Guarantees and their conditions
//!
//! Given a store meeting contract C1–C8 (see [`ReservationStore`]):
//!
//! - [`GenerationReserver::open`] makes exactly two reads and never writes. It
//!   never creates, repairs, provisions or issues.
//! - A [`ReservedGeneration`] is returned only at or below a ceiling that the
//!   same reserver instance wrote to **both** slots and read back byte-identical.
//!   The first `reserve` after every `open` therefore always commits:
//!   "reserve before activation" on every startup.
//! - Each commit writes first a slot that is **not** a confirmed current copy, so
//!   at every instant an untouched slot holds a record at or above every issued
//!   value. Open selects the maximum, so a single torn, erased, rotted, marginal
//!   or unreadable slot, or a power loss at any point, never lowers the next value.
//! - Blank, corrupt, below-floor, rolled-back (witness) and exhausted storage fail
//!   closed with a typed [`Remedy`]; only out-of-band trusted provisioning with a
//!   new, registry-issued epoch recovers.
//!
//! A faithful restore of **both** slots is detected only as declared by the
//! required [`RollbackDefense`]. A store that acknowledges writes it did not make
//! durable, or that violates C2/C3/C5/C8, is outside these guarantees.
//!
//! # Example
//!
//! ```
//! use core::num::NonZeroU32;
//! use neuradix_command_core::ExecutionMode;
//! use neuradix_command_core::reservation::{
//!     BindingKey, GenerationReserver, NamespaceEpoch, ProvisionGuards, RECORD_BYTES,
//!     ReceiverId, ReservationKey, ReservationStore, ReserverConfig, RollbackDefense, Slot,
//!     StoreError, provision,
//! };
//!
//! // Volatile stand-in for the doctest only: NOT a durable store.
//! struct Ram([[u8; RECORD_BYTES]; 2]);
//! impl ReservationStore for Ram {
//!     fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
//!         *buf = self.0[slot.index()];
//!         Ok(())
//!     }
//!     fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
//!         self.0[slot.index()] = *record;
//!         Ok(())
//!     }
//! }
//!
//! let mut store = Ram([[0xFF; RECORD_BYTES]; 2]);
//! let receiver = ReceiverId::new([7; 16]).unwrap();
//! let key = ReservationKey::new(receiver, BindingKey::numeric(1, 2, 7, ExecutionMode::Live));
//! let config = ReserverConfig::new(NonZeroU32::new(4).unwrap(), RollbackDefense::Unprotected);
//!
//! // A blank store fails closed: nothing is issued until trusted provisioning.
//! let refused = GenerationReserver::open(&mut store, key, config).unwrap_err();
//! assert_eq!(refused.error.remedy(), neuradix_command_core::reservation::Remedy::Provision);
//!
//! // Trusted, out-of-band provisioning (feature `provisioning`), epoch from a registry.
//! provision(&mut store, key, NamespaceEpoch::new(1).unwrap(), ProvisionGuards::default()).unwrap();
//!
//! let mut reserver = GenerationReserver::open(&mut store, key, config).map_err(|f| f.error).unwrap();
//! let token = reserver.reserve().unwrap();          // committed to both slots first
//! assert_eq!(token.generation().get(), (1u128 << 64) | 1);
//! let next = reserver.reserve_from_window().unwrap(); // inside the window: no store calls
//! assert!(next.generation() > token.generation());
//! ```
//!
//! # Compile-time ownership checks
//!
//! Each `compile_fail` block below has a passing twin with identical scaffolding.
//!
//! A token has no public constructor, and no conversion from a plain generation:
//!
//! ```compile_fail
//! use neuradix_command_core::reservation::ReservedGeneration;
//! let token = ReservedGeneration { generation: todo!(), key: todo!(), epoch: todo!() };
//! ```
//!
//! ```compile_fail
//! use neuradix_command_core::{Generation, reservation::ReservedGeneration};
//! let g = Generation::new(1).unwrap();
//! let token: ReservedGeneration = g.into();
//! ```
//!
//! ```
//! use neuradix_command_core::{Generation, reservation::ReservedGeneration};
//! let g = Generation::new(1).unwrap();
//! let _plain: Generation = g;
//! fn _requires_token(_: ReservedGeneration) {}
//! ```
//!
//! A token cannot be duplicated:
//!
//! ```compile_fail
//! use neuradix_command_core::reservation::ReservedGeneration;
//! fn duplicate(token: &ReservedGeneration) -> ReservedGeneration {
//!     token.clone()
//! }
//! ```
//!
//! ```
//! use neuradix_command_core::{Generation, reservation::ReservedGeneration};
//! fn read(token: &ReservedGeneration) -> Generation {
//!     token.generation()
//! }
//! ```
//!
//! Two reservers cannot share one exclusively borrowed store, a reserver cannot
//! be cloned, and `reserve` requires exclusive access:
//!
//! ```compile_fail
//! # use neuradix_command_core::reservation::*;
//! # struct Ram([[u8; RECORD_BYTES]; 2]);
//! # impl ReservationStore for Ram {
//! #     fn read(&mut self, s: Slot, b: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> { *b = self.0[s.index()]; Ok(()) }
//! #     fn write(&mut self, s: Slot, r: &[u8; RECORD_BYTES]) -> Result<(), StoreError> { self.0[s.index()] = *r; Ok(()) }
//! # }
//! fn two(store: &mut Ram, key: ReservationKey, config: ReserverConfig) {
//!     let a = GenerationReserver::open(&mut *store, key, config);
//!     let b = GenerationReserver::open(&mut *store, key, config);
//!     drop((a, b));
//! }
//! ```
//!
//! ```
//! # use neuradix_command_core::reservation::*;
//! # struct Ram([[u8; RECORD_BYTES]; 2]);
//! # impl ReservationStore for Ram {
//! #     fn read(&mut self, s: Slot, b: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> { *b = self.0[s.index()]; Ok(()) }
//! #     fn write(&mut self, s: Slot, r: &[u8; RECORD_BYTES]) -> Result<(), StoreError> { self.0[s.index()] = *r; Ok(()) }
//! # }
//! fn one_then_another(store: &mut Ram, key: ReservationKey, config: ReserverConfig) {
//!     let a = GenerationReserver::open(&mut *store, key, config);
//!     drop(a);
//!     let b = GenerationReserver::open(&mut *store, key, config);
//!     drop(b);
//! }
//! ```
//!
//! ```compile_fail
//! # use neuradix_command_core::reservation::*;
//! fn fork<S: ReservationStore>(r: &GenerationReserver<S>) -> GenerationReserver<S> {
//!     r.clone()
//! }
//! ```
//!
//! ```
//! # use neuradix_command_core::reservation::*;
//! fn inspect<S: ReservationStore>(r: &GenerationReserver<S>) -> ReserverStatus {
//!     r.status()
//! }
//! ```
//!
//! ```compile_fail
//! # use neuradix_command_core::reservation::*;
//! fn shared<S: ReservationStore>(r: &GenerationReserver<S>) {
//!     let _burned = r.reserve();
//! }
//! ```
//!
//! ```
//! # use neuradix_command_core::reservation::*;
//! fn exclusive<S: ReservationStore>(r: &mut GenerationReserver<S>) {
//!     let _burned = r.reserve();
//! }
//! ```

use core::num::{NonZeroU32, NonZeroU64};

use crate::{ExecutionMode, Generation};

pub mod record;
use record::{RecordFields, SlotView};

#[cfg(feature = "provisioning")]
mod provisioning;
#[cfg(feature = "provisioning")]
pub use provisioning::{ProvisionError, ProvisionGuards, ProvisionReport, provision};

/// Size of one slot record in bytes.
pub const RECORD_BYTES: usize = 64;
/// Record magic, bytes 0..4.
pub const RECORD_MAGIC: [u8; 4] = *b"NRXG";
/// Record format version understood by this build.
pub const RECORD_FORMAT: u8 = 1;
/// `open`: exactly two reads (A then B) on every outcome; never a write.
pub const MAX_STORE_CALLS_PER_OPEN: u32 = 2;
/// `reserve`: 0 inside the window; otherwise at most 2 pre-commit reads, 2 writes
/// and 2 read-backs. `reserve_from_window` and `status` always make 0 calls.
pub const MAX_STORE_CALLS_PER_RESERVE: u32 = 6;
/// `provision`: at most 2 reads, 2 writes and 2 read-backs; every refusal is
/// exactly 2 reads and 0 writes.
pub const MAX_STORE_CALLS_PER_PROVISION: u32 = 6;

/// One of the two mirrored record slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// First slot.
    A,
    /// Second slot, in an independent erase/program unit.
    B,
}
impl Slot {
    /// Array index: A = 0, B = 1.
    pub const fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
    /// The other slot.
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
    pub(crate) const fn tag(self) -> u8 {
        self.index() as u8
    }
}

/// Store call failure.
///
/// - `read`: `Corrupt` is an integrity failure confined to the addressed slot (for
///   example uncorrectable ECC) and is treated like a CRC failure; `Io` is a
///   transient failure with unknown content (the slot is unreadable);
///   `Unavailable` means this handle is unusable until re-acquired.
/// - `write`: every variant means the slot may be unchanged, erased, torn,
///   marginal or new, and poisons the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    /// Integrity failure confined to the addressed slot.
    Corrupt,
    /// Transient failure; content unknown.
    Io,
    /// This handle is unusable (poisoned, closed or replaced).
    Unavailable,
}
impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for StoreError {}

/// Durable storage for ONE [`ReservationKey`]: two slots of [`RECORD_BYTES`].
///
/// Normative contract, met by board support or the host store. The library
/// cannot detect violations:
///
/// - **C1** `write` returning `Ok` means durable: after any power loss, `read` of
///   that slot returns exactly those bytes until the next write of that slot.
/// - **C2** A failed or interrupted write affects ONLY the addressed slot. A and B
///   sit in independent erase/program units: never two halves of one erase
///   sector, page, ECC word or file.
/// - **C3** `read` returns medium content, never a volatile write-back or XIP cache.
/// - **C4** No deferred or background writes after a call returns.
/// - **C5** One store object per physical slot pair and one store per (receiver,
///   binding). Store types must NOT be `Clone`/`Copy`, and no second handle may
///   alias the region.
/// - **C6** A never-written or erased slot reads as all-0xFF or all-0x00, or as
///   anything failing the CRC.
/// - **C7** Synchronous; returns within the deployment's admitted budget; no
///   executor, HAL or transport types in the portable API.
/// - **C8** The region is never part of any firmware, factory or update image,
///   backup, snapshot or programmer dump that can be written back. A reflash
///   either preserves it or erases it; writing back an old copy is a rollback.
///
/// Any implementation, including log-structured or wear-levelled ones, must pass
/// the WP-A04.4 fault-injection qualification suite before it is claimed.
pub trait ReservationStore {
    /// Read the whole slot into `buf`.
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError>;
    /// Replace the whole slot with `record` durably (C1) before returning `Ok`.
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError>;
}
impl<S: ReservationStore + ?Sized> ReservationStore for &mut S {
    fn read(&mut self, slot: Slot, buf: &mut [u8; RECORD_BYTES]) -> Result<(), StoreError> {
        (**self).read(slot, buf)
    }
    fn write(&mut self, slot: Slot, record: &[u8; RECORD_BYTES]) -> Result<(), StoreError> {
        (**self).write(slot, record)
    }
}

/// Device-unique, non-secret receiver identity taken from OUTSIDE the reservation
/// storage image (MCU unique ID or OTP, or host provisioning configuration).
/// Not a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceiverId([u8; 16]);
impl ReceiverId {
    /// `None` for all-0x00 or all-0xFF, which are unprogrammed OTP patterns.
    pub const fn new(bytes: [u8; 16]) -> Option<Self> {
        let mut zero = true;
        let mut ones = true;
        let mut i = 0;
        while i < 16 {
            zero &= bytes[i] == 0x00;
            ones &= bytes[i] == 0xFF;
            i += 1;
        }
        if zero || ones {
            None
        } else {
            Some(Self(bytes))
        }
    }
    /// Raw identity bytes.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Accidental mix-up detector for an actuator binding. NOT a credential and not
/// forgery-resistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingKey(u64);
impl BindingKey {
    /// Wrap a raw key.
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
    /// Raw key.
    pub const fn get(self) -> u64 {
        self.0
    }
    /// FNV-1a-64 over the domain and then each part, every item prefixed by its
    /// length as u64 little-endian, so part boundaries cannot be shifted.
    pub const fn derive(domain: &[u8], parts: &[&[u8]]) -> Self {
        let mut hash = fnv_item(FNV_OFFSET, domain);
        let mut i = 0;
        while i < parts.len() {
            hash = fnv_item(hash, parts[i]);
            i += 1;
        }
        Self(hash)
    }
    /// Key for a numeric (embedded) binding.
    pub const fn numeric(holder: u64, capability: u64, endpoint: u64, mode: ExecutionMode) -> Self {
        Self::derive(
            b"neuradix.actuator-binding.numeric.v1",
            &[
                &holder.to_le_bytes(),
                &capability.to_le_bytes(),
                &endpoint.to_le_bytes(),
                &[mode_tag(mode)],
            ],
        )
    }
    /// Key for a named (host) binding.
    pub const fn named(holder: &str, capability: &str, driver: &str, mode: ExecutionMode) -> Self {
        Self::derive(
            b"neuradix.actuator-binding.named.v1",
            &[
                holder.as_bytes(),
                capability.as_bytes(),
                driver.as_bytes(),
                &[mode_tag(mode)],
            ],
        )
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

const fn fnv_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

const fn fnv_item(hash: u64, bytes: &[u8]) -> u64 {
    fnv_bytes(fnv_bytes(hash, &(bytes.len() as u64).to_le_bytes()), bytes)
}

const fn mode_tag(mode: ExecutionMode) -> u8 {
    match mode {
        ExecutionMode::Live => 1,
        ExecutionMode::Simulation => 2,
        ExecutionMode::Replay => 3,
    }
}

/// The receiver and binding a store belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservationKey {
    receiver: ReceiverId,
    binding: BindingKey,
}
impl ReservationKey {
    /// Pair a receiver identity with a binding key.
    pub const fn new(receiver: ReceiverId, binding: BindingKey) -> Self {
        Self { receiver, binding }
    }
    /// Receiver identity.
    pub const fn receiver(&self) -> ReceiverId {
        self.receiver
    }
    /// Binding key.
    pub const fn binding(&self) -> BindingKey {
        self.binding
    }
}

/// High 64 bits of every reserved generation. Issued only by trusted provisioning
/// from a durable, fleet-wide, strictly increasing, single-use registry. Never
/// learned from storage or traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamespaceEpoch(NonZeroU64);
impl NamespaceEpoch {
    /// `None` for zero.
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
    pub(crate) const fn from_nonzero(value: NonZeroU64) -> Self {
        Self(value)
    }
    /// Raw epoch.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
    /// The epoch of a reserved-layout generation; `None` below 2^64 (legacy,
    /// fixture, simulation and replay space).
    pub const fn of(generation: Generation) -> Option<Self> {
        Self::new((generation.get() >> 64) as u64)
    }
}

/// REQUIRED rollback posture; there is no default. Floor and witness only ever
/// refuse; they never select or raise an allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackDefense {
    /// Trusted lowest acceptable epoch (OTP counter, secure element, or registry
    /// configuration that cannot be restored with the image). Detects a restore
    /// ACROSS an epoch boundary; does NOT detect a restore within the epoch.
    Floor(NamespaceEpoch),
    /// Highest generation ever handed to any authorized source for this key, read
    /// from a trusted durable record outside the reservation image. `open` refuses
    /// if the stored ceiling is below it. An untrusted value is harmless for
    /// safety but can deny service.
    Witness(Generation),
    /// Both checks.
    FloorAndWitness {
        /// Lowest acceptable epoch.
        floor: NamespaceEpoch,
        /// Highest generation previously provisioned to a source.
        witness: Generation,
    },
    /// Explicit acknowledgement that a faithful restore of both slots is NOT
    /// detected. The WP-A04.2 rollback obligation then rests on deployment
    /// controls (C8, a new epoch after any restore). Reported in status.
    Unprotected,
}
impl RollbackDefense {
    /// Configured floor, if any.
    pub const fn floor(self) -> Option<NamespaceEpoch> {
        match self {
            Self::Floor(floor) | Self::FloorAndWitness { floor, .. } => Some(floor),
            Self::Witness(_) | Self::Unprotected => None,
        }
    }
    /// Configured witness, if any.
    pub const fn witness(self) -> Option<Generation> {
        match self {
            Self::Witness(witness) | Self::FloorAndWitness { witness, .. } => Some(witness),
            Self::Floor(_) | Self::Unprotected => None,
        }
    }
    /// Reportable posture without the configured values.
    pub const fn posture(self) -> RollbackPosture {
        match self {
            Self::Floor(_) => RollbackPosture::Floor,
            Self::Witness(_) => RollbackPosture::Witness,
            Self::FloorAndWitness { .. } => RollbackPosture::FloorAndWitness,
            Self::Unprotected => RollbackPosture::Unprotected,
        }
    }
}

/// Rollback posture reported by [`ReserverStatus`]; trusted setup must log it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackPosture {
    /// Cross-epoch restores detected.
    Floor,
    /// Restores below the witness detected, within or across epochs.
    Witness,
    /// Both.
    FloorAndWitness,
    /// Faithful two-slot restores are not detected.
    Unprotected,
}
impl RollbackPosture {
    /// Whether a faithful restore within the current epoch is detected.
    pub const fn detects_restore_within_epoch(self) -> bool {
        matches!(self, Self::Witness | Self::FloorAndWitness)
    }
}

/// Reserver configuration. No `Default`: trusted setup must choose a window and
/// a rollback posture explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReserverConfig {
    window: NonZeroU32,
    rollback: RollbackDefense,
}
impl ReserverConfig {
    /// `window` is the number of generations covered by one durable commit
    /// (flash wear versus values burned by a restart).
    pub const fn new(window: NonZeroU32, rollback: RollbackDefense) -> Self {
        Self { window, rollback }
    }
    /// Generations covered by one commit.
    pub const fn window(self) -> NonZeroU32 {
        self.window
    }
    /// Declared rollback defense.
    pub const fn rollback(self) -> RollbackDefense {
        self.rollback
    }
}

/// Proof that [`generation`](Self::generation) lies at or below a ceiling THIS
/// reserver instance wrote to both slots and read back byte-identical, via a store
/// assumed to meet C1–C8.
///
/// Not `Clone`, `Copy`, `Default` or `PartialEq`; private fields; no public
/// constructor and no conversion from [`Generation`]. It is consumed by at most
/// one permission; dropping it burns the value, which is never issued again.
#[must_use = "dropping a reservation burns its generation"]
#[derive(Debug)]
pub struct ReservedGeneration {
    generation: Generation,
    key: ReservationKey,
    epoch: NamespaceEpoch,
}
impl ReservedGeneration {
    /// `(epoch << 64) | counter`, with counter >= 1.
    pub const fn generation(&self) -> Generation {
        self.generation
    }
    /// Receiver and binding this value was reserved for.
    pub const fn key(&self) -> ReservationKey {
        self.key
    }
    /// Namespace epoch of the value.
    pub const fn epoch(&self) -> NamespaceEpoch {
        self.epoch
    }
}

/// Observed condition of a slot at open or at the last commit attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotCondition {
    /// Holds the current ceiling.
    Current,
    /// Holds an older record for this key.
    Stale,
    /// Erased.
    Blank,
    /// Failed integrity or field checks.
    Corrupt,
    /// Read returned a transient error.
    Unreadable,
}

/// Why a reserver stopped issuing. Permanent for the instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoisonCause {
    /// A slot write returned an error.
    WriteFailed(Slot),
    /// A read-back failed or differed from the written bytes.
    VerifyFailed(Slot),
    /// A pre-commit read failed while no slot was confirmed current, or the
    /// handle became unavailable.
    ReadFailed(Slot),
    /// Pre-commit content contradicted this instance's state (external writer,
    /// restore, foreign or unsupported record).
    MediaChanged,
}

/// Snapshot of reserver state; costs no store calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReserverStatus {
    /// Receiver and binding.
    pub key: ReservationKey,
    /// Namespace epoch in force.
    pub epoch: NamespaceEpoch,
    /// `(epoch << 64) | persisted high-water counter`.
    pub durable_ceiling: u128,
    /// Values issuable with zero store calls.
    pub window_remaining: u64,
    /// Slot conditions as last observed (open, or the last commit attempt).
    pub slots: [SlotCondition; 2],
    /// Saturating wear diagnostic; never used for ordering.
    pub commits: u32,
    /// Set once a commit failed or storage contradicted this instance.
    pub poisoned: Option<PoisonCause>,
    /// Declared rollback posture; trusted setup must log it.
    pub rollback: RollbackPosture,
}

/// Operator action for a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remedy {
    /// Open again with the same handle (read-only; always permitted).
    RetryOpen,
    /// Drop the reserver and the handle, re-acquire a fresh handle (host: a new
    /// store object; MCU: re-initialize the peripheral or reset), then open.
    /// Strict deployments may Provision instead.
    ReopenStore,
    /// Trusted, out-of-band provisioning with a new registry epoch; never from the
    /// boot path.
    Provision,
    /// Trusted maintenance (erase or rewire), then Provision.
    RepairStore,
    /// Call `reserve` before ingress, after `revoke` applied the safe output, or
    /// from a context that cannot delay evaluation ticks.
    CommitWhenSafe,
}

/// Why `open` refused. Every refusal made exactly two reads and zero writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// A read returned `Unavailable`: the handle is unusable.
    StoreUnavailable,
    /// A CRC-valid slot has an unknown version, flags or reserved bytes.
    UnsupportedFormat,
    /// A record's slot tag differs from the slot read.
    SlotAliasing,
    /// A valid record belongs to another receiver (cloned image or wiring).
    ForeignReceiver,
    /// A valid record belongs to another binding (wiring).
    ForeignBinding,
    /// No record for this key, and at least one read returned `Io`.
    Unreadable,
    /// Both slots erased: new device or lost storage.
    Blank,
    /// No valid record and no unreadable slot.
    Corrupt,
    /// The stored epoch is below the trusted floor.
    BelowFloor {
        /// Epoch found in storage.
        stored: NamespaceEpoch,
        /// Configured floor.
        floor: NamespaceEpoch,
    },
    /// The stored ceiling is below the trusted witness.
    RolledBack {
        /// Configured witness.
        witness: Generation,
        /// `(epoch << 64) | high_water` found in storage.
        stored: u128,
    },
    /// The counter reached `u64::MAX`; the epoch is terminal.
    Exhausted,
}
impl OpenError {
    /// Operator action.
    pub const fn remedy(self) -> Remedy {
        match self {
            Self::StoreUnavailable => Remedy::ReopenStore,
            Self::UnsupportedFormat
            | Self::SlotAliasing
            | Self::ForeignReceiver
            | Self::ForeignBinding => Remedy::RepairStore,
            Self::Unreadable => Remedy::RetryOpen,
            Self::Blank
            | Self::Corrupt
            | Self::BelowFloor { .. }
            | Self::RolledBack { .. }
            | Self::Exhausted => Remedy::Provision,
        }
    }
}
impl core::fmt::Display for OpenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for OpenError {}

/// Why `reserve` or `reserve_from_window` returned no value. Nothing was issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveError {
    /// This call poisoned the reserver; the commit outcome is uncertain.
    Uncertain(PoisonCause),
    /// An earlier call poisoned the reserver.
    Poisoned(PoisonCause),
    /// The epoch's counter space is used up.
    Exhausted,
    /// `reserve_from_window` only: a durable commit would be required.
    CommitRequired,
}
impl ReserveError {
    /// Operator action.
    pub const fn remedy(self) -> Remedy {
        match self {
            Self::Uncertain(_) | Self::Poisoned(_) => Remedy::ReopenStore,
            Self::Exhausted => Remedy::Provision,
            Self::CommitRequired => Remedy::CommitWhenSafe,
        }
    }
}
impl core::fmt::Display for ReserveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for ReserveError {}

/// A refused open returns the store, so an MCU peripheral singleton is never lost.
pub struct OpenFailure<S> {
    /// Why open refused.
    pub error: OpenError,
    /// The store, unchanged by open.
    pub store: S,
}
impl<S> OpenFailure<S> {
    /// Split into error and store.
    pub fn into_parts(self) -> (OpenError, S) {
        (self.error, self.store)
    }
}
impl<S> core::fmt::Debug for OpenFailure<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl<S> core::fmt::Display for OpenFailure<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.error)
    }
}
impl<S> core::error::Error for OpenFailure<S> {}

/// Classification of one slot read against a key.
#[derive(Debug, Clone, Copy)]
pub(crate) enum View {
    Blank,
    /// Decoded as corrupt (CRC, magic or field values).
    Corrupt,
    /// `read` returned `Err(Corrupt)`.
    ReadCorrupt,
    /// `read` returned `Err(Io)`.
    Unreadable,
    /// `read` returned `Err(Unavailable)`.
    Unavailable,
    Unsupported,
    Aliased,
    Foreign(RecordFields),
    Mine(RecordFields),
}
impl View {
    fn read_error(&self) -> bool {
        matches!(
            self,
            Self::ReadCorrupt | Self::Unreadable | Self::Unavailable
        )
    }
}

/// Read and classify one slot relative to `key`. Foreign records keep their
/// fields so callers can distinguish receiver from binding mismatches.
pub(crate) fn read_view<S: ReservationStore + ?Sized>(
    store: &mut S,
    slot: Slot,
    key: ReservationKey,
    buf: &mut [u8; RECORD_BYTES],
) -> View {
    match store.read(slot, buf) {
        Err(StoreError::Corrupt) => View::ReadCorrupt,
        Err(StoreError::Io) => View::Unreadable,
        Err(StoreError::Unavailable) => View::Unavailable,
        Ok(()) => match record::decode(buf) {
            SlotView::Blank => View::Blank,
            SlotView::Corrupt => View::Corrupt,
            SlotView::Unsupported => View::Unsupported,
            SlotView::Valid(fields) if fields.slot != slot => View::Aliased,
            SlotView::Valid(fields) if fields.key == key => View::Mine(fields),
            SlotView::Valid(fields) => View::Foreign(fields),
        },
    }
}

pub(crate) const fn rank(fields: &RecordFields) -> (u64, u64, u32) {
    (fields.epoch.get(), fields.high_water, fields.commits)
}

/// Write `fields` to `slot` and read it back byte-exact. Stops at the first error.
pub(crate) fn verified_write<S: ReservationStore + ?Sized>(
    store: &mut S,
    fields: &RecordFields,
    buf: &mut [u8; RECORD_BYTES],
) -> Result<(), PoisonCause> {
    let slot = fields.slot;
    let bytes = record::encode(fields);
    if store.write(slot, &bytes).is_err() {
        return Err(PoisonCause::WriteFailed(slot));
    }
    match store.read(slot, buf) {
        Ok(()) if *buf == bytes => Ok(()),
        _ => Err(PoisonCause::VerifyFailed(slot)),
    }
}

/// One allocator per (receiver, binding) store, owned by trusted setup for the
/// process lifetime. Not `Clone`. Owns its store by value; pass `&mut store` to
/// keep ownership. Two reservers over one store are a borrow error when the store
/// type is not `Clone`/`Copy` (C5).
pub struct GenerationReserver<S: ReservationStore> {
    store: S,
    key: ReservationKey,
    epoch: NamespaceEpoch,
    ceiling: u64,
    commits: u32,
    next: Option<u64>,
    window: NonZeroU32,
    slots: [SlotCondition; 2],
    poisoned: Option<PoisonCause>,
    rollback: RollbackPosture,
}

impl<S: ReservationStore> core::fmt::Debug for GenerationReserver<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GenerationReserver")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl<S: ReservationStore> GenerationReserver<S> {
    /// Startup: exactly two reads (A then B) and NEVER a write. Never creates,
    /// repairs, provisions or issues. Refusal precedence: unavailable handle,
    /// unsupported format, slot aliasing, foreign receiver, foreign binding, then
    /// no record for the key (unreadable, blank or corrupt), floor, witness and
    /// exhaustion.
    pub fn open(
        mut store: S,
        key: ReservationKey,
        config: ReserverConfig,
    ) -> Result<Self, OpenFailure<S>> {
        let mut buf = [0u8; RECORD_BYTES];
        let views = [
            read_view(&mut store, Slot::A, key, &mut buf),
            read_view(&mut store, Slot::B, key, &mut buf),
        ];
        match select(key, views, config.rollback) {
            Ok((best, slots)) => Ok(Self {
                store,
                key,
                epoch: best.epoch,
                ceiling: best.high_water,
                commits: best.commits,
                // high_water < u64::MAX here, so the first reserve always commits.
                next: best.high_water.checked_add(1),
                window: config.window,
                slots,
                poisoned: None,
                rollback: config.rollback.posture(),
            }),
            Err(error) => Err(OpenFailure { error, store }),
        }
    }

    /// Return a value only after its ceiling was durably committed by THIS
    /// instance (both slots written and read back byte-identical). The first call
    /// after `open` always commits. Store calls: 0 inside the window, otherwise
    /// exactly 6 on success and at most 6 on failure. A failure poisons the
    /// instance permanently and issues nothing.
    ///
    /// Scheduling: a committing call may block for erase/program time. Call it
    /// before ingress, after `revoke` applied the safe output, or from a context
    /// that cannot delay evaluation ticks; use
    /// [`reserve_from_window`](Self::reserve_from_window) while an adapter is granted.
    pub fn reserve(&mut self) -> Result<ReservedGeneration, ReserveError> {
        if let Some(cause) = self.poisoned {
            return Err(ReserveError::Poisoned(cause));
        }
        let Some(n) = self.next else {
            return Err(ReserveError::Exhausted);
        };
        if n > self.ceiling
            && let Err(cause) = self.commit(n)
        {
            self.poisoned = Some(cause);
            return Err(ReserveError::Uncertain(cause));
        }
        self.issue(n)
    }

    /// Issue from the already committed window with zero store calls.
    /// `CommitRequired` (not a poison) when a commit would be needed, which is
    /// always the case right after `open` and at the end of each window.
    pub fn reserve_from_window(&mut self) -> Result<ReservedGeneration, ReserveError> {
        if let Some(cause) = self.poisoned {
            return Err(ReserveError::Poisoned(cause));
        }
        let Some(n) = self.next else {
            return Err(ReserveError::Exhausted);
        };
        if n > self.ceiling {
            return Err(ReserveError::CommitRequired);
        }
        self.issue(n)
    }

    /// State snapshot; zero store calls.
    pub fn status(&self) -> ReserverStatus {
        let window_remaining = match self.next {
            Some(n) if n <= self.ceiling => self.ceiling - n + 1,
            _ => 0,
        };
        ReserverStatus {
            key: self.key,
            epoch: self.epoch,
            durable_ceiling: compose(self.epoch, self.ceiling),
            window_remaining,
            slots: self.slots,
            commits: self.commits,
            poisoned: self.poisoned,
            rollback: self.rollback,
        }
    }

    /// Release the store. Unused window values are burned.
    pub fn into_store(self) -> S {
        self.store
    }

    fn issue(&mut self, n: u64) -> Result<ReservedGeneration, ReserveError> {
        let Some(generation) = Generation::new(compose(self.epoch, n)) else {
            // Unreachable: the epoch is nonzero.
            return Err(ReserveError::Exhausted);
        };
        self.next = n.checked_add(1);
        Ok(ReservedGeneration {
            generation,
            key: self.key,
            epoch: self.epoch,
        })
    }

    /// Commit a new ceiling covering `n`: a pre-commit read of both slots, then a
    /// verified write of a non-current slot first and the other slot second.
    fn commit(&mut self, n: u64) -> Result<(), PoisonCause> {
        let target = n.saturating_add(u64::from(self.window.get() - 1));
        let mut buf = [0u8; RECORD_BYTES];
        let views = [
            read_view(&mut self.store, Slot::A, self.key, &mut buf),
            read_view(&mut self.store, Slot::B, self.key, &mut buf),
        ];
        let current = (self.epoch.get(), self.ceiling);
        let mut is_current = [false; 2];
        for (i, view) in views.iter().enumerate() {
            self.slots[i] = match view {
                View::Mine(f) if (f.epoch.get(), f.high_water) == current => {
                    is_current[i] = true;
                    SlotCondition::Current
                }
                View::Mine(_) => SlotCondition::Stale,
                View::Blank => SlotCondition::Blank,
                View::Unreadable | View::Unavailable => SlotCondition::Unreadable,
                _ => SlotCondition::Corrupt,
            };
        }
        if let Some(i) = views.iter().position(|v| matches!(v, View::Unavailable)) {
            return Err(PoisonCause::ReadFailed(SLOTS[i]));
        }
        let contradicts = views.iter().any(|view| match view {
            View::Unsupported | View::Aliased | View::Foreign(_) => true,
            View::Mine(f) => (f.epoch.get(), f.high_water) > current,
            _ => false,
        });
        if contradicts {
            return Err(PoisonCause::MediaChanged);
        }
        let first = match is_current {
            [false, false] => {
                return Err(match views.iter().position(View::read_error) {
                    Some(i) => PoisonCause::ReadFailed(SLOTS[i]),
                    None => PoisonCause::MediaChanged,
                });
            }
            [true, true] => Slot::A,
            [true, false] => Slot::B,
            [false, true] => Slot::A,
        };
        let commits = self.commits.saturating_add(1);
        for slot in [first, first.other()] {
            let fields = RecordFields {
                slot,
                key: self.key,
                epoch: self.epoch,
                high_water: target,
                commits,
            };
            verified_write(&mut self.store, &fields, &mut buf)?;
            self.slots[slot.index()] = SlotCondition::Current;
        }
        self.ceiling = target;
        self.commits = commits;
        Ok(())
    }
}

const SLOTS: [Slot; 2] = [Slot::A, Slot::B];

pub(crate) const fn compose(epoch: NamespaceEpoch, counter: u64) -> u128 {
    ((epoch.get() as u128) << 64) | counter as u128
}

/// Apply the open precedence and checks to two classified slots.
fn select(
    key: ReservationKey,
    views: [View; 2],
    rollback: RollbackDefense,
) -> Result<(RecordFields, [SlotCondition; 2]), OpenError> {
    let any = |pred: &dyn Fn(&View) -> bool| views.iter().any(pred);
    if any(&|v| matches!(v, View::Unavailable)) {
        return Err(OpenError::StoreUnavailable);
    }
    if any(&|v| matches!(v, View::Unsupported)) {
        return Err(OpenError::UnsupportedFormat);
    }
    if any(&|v| matches!(v, View::Aliased)) {
        return Err(OpenError::SlotAliasing);
    }
    if any(&|v| matches!(v, View::Foreign(f) if f.key.receiver() != key.receiver())) {
        return Err(OpenError::ForeignReceiver);
    }
    if any(&|v| matches!(v, View::Foreign(_))) {
        return Err(OpenError::ForeignBinding);
    }
    let mut best: Option<RecordFields> = None;
    for view in &views {
        if let View::Mine(fields) = view
            && best.is_none_or(|b| rank(fields) > rank(&b))
        {
            best = Some(*fields);
        }
    }
    let Some(best) = best else {
        return Err(if any(&|v| matches!(v, View::Unreadable)) {
            OpenError::Unreadable
        } else if any(&|v| !matches!(v, View::Blank)) {
            OpenError::Corrupt
        } else {
            OpenError::Blank
        });
    };
    if let Some(floor) = rollback.floor()
        && best.epoch < floor
    {
        return Err(OpenError::BelowFloor {
            stored: best.epoch,
            floor,
        });
    }
    let stored = compose(best.epoch, best.high_water);
    if let Some(witness) = rollback.witness()
        && witness.get() > stored
    {
        return Err(OpenError::RolledBack { witness, stored });
    }
    if best.high_water == u64::MAX {
        return Err(OpenError::Exhausted);
    }
    let condition = |view: &View| match view {
        View::Mine(f) if (f.epoch, f.high_water) == (best.epoch, best.high_water) => {
            SlotCondition::Current
        }
        View::Mine(_) => SlotCondition::Stale,
        View::Blank => SlotCondition::Blank,
        View::Unreadable => SlotCondition::Unreadable,
        _ => SlotCondition::Corrupt,
    };
    Ok((best, [condition(&views[0]), condition(&views[1])]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u1_crc_check_value_including_const_context() {
        const CHECK: u32 = record::crc32(b"123456789");
        assert_eq!(CHECK, 0xCBF4_3926);
        assert_eq!(record::crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn u2_derive_is_length_delimited() {
        let ab_c = BindingKey::derive(b"d", &[b"a", b"bc"]);
        let a_bc = BindingKey::derive(b"d", &[b"ab", b"c"]);
        assert_eq!(ab_c.get(), 0x0b50_5a6c_7bb8_e219);
        assert_eq!(a_bc.get(), 0x4c35_4686_0759_f7d9);
        assert_ne!(ab_c, a_bc);
    }

    #[test]
    fn u3_binding_key_goldens_and_mode_separation() {
        use ExecutionMode::*;
        assert_eq!(
            BindingKey::numeric(1, 2, 7, Live).get(),
            0x4bf1_57b8_d1af_9724
        );
        assert_eq!(
            BindingKey::numeric(1, 2, 7, Simulation).get(),
            0x4bf1_5ab8_d1af_9c3d
        );
        assert_eq!(
            BindingKey::numeric(1, 2, 8, Live).get(),
            0x5f8b_4b41_d4df_c459
        );
        assert_eq!(
            BindingKey::named("controller", "thrust", "driver/one", Live).get(),
            0x2249_4187_5563_01ab
        );
        let modes = [Live, Simulation, Replay].map(|m| BindingKey::numeric(1, 2, 7, m));
        assert_ne!(modes[0], modes[1]);
        assert_ne!(modes[1], modes[2]);
        assert_ne!(modes[0], modes[2]);
    }

    #[test]
    fn u4_receiver_id_rejects_unprogrammed_patterns() {
        assert!(ReceiverId::new([0x00; 16]).is_none());
        assert!(ReceiverId::new([0xFF; 16]).is_none());
        let mut bytes = [0u8; 16];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8 + 1;
        }
        assert_eq!(ReceiverId::new(bytes).unwrap().as_bytes(), &bytes);
    }

    #[test]
    fn u5_namespace_epoch() {
        assert!(NamespaceEpoch::new(0).is_none());
        let below = Generation::new((1u128 << 64) - 1).unwrap();
        assert!(NamespaceEpoch::of(below).is_none());
        let g = Generation::new((3u128 << 64) | 5).unwrap();
        assert_eq!(NamespaceEpoch::of(g).unwrap().get(), 3);
    }

    #[test]
    fn u6_remedy_table() {
        let epoch = NamespaceEpoch::new(1).unwrap();
        let g = Generation::new(1).unwrap();
        let open = |e: OpenError| match e {
            OpenError::StoreUnavailable => Remedy::ReopenStore,
            OpenError::UnsupportedFormat
            | OpenError::SlotAliasing
            | OpenError::ForeignReceiver
            | OpenError::ForeignBinding => Remedy::RepairStore,
            OpenError::Unreadable => Remedy::RetryOpen,
            OpenError::Blank
            | OpenError::Corrupt
            | OpenError::BelowFloor { .. }
            | OpenError::RolledBack { .. }
            | OpenError::Exhausted => Remedy::Provision,
        };
        for e in [
            OpenError::StoreUnavailable,
            OpenError::UnsupportedFormat,
            OpenError::SlotAliasing,
            OpenError::ForeignReceiver,
            OpenError::ForeignBinding,
            OpenError::Unreadable,
            OpenError::Blank,
            OpenError::Corrupt,
            OpenError::BelowFloor {
                stored: epoch,
                floor: epoch,
            },
            OpenError::RolledBack {
                witness: g,
                stored: 0,
            },
            OpenError::Exhausted,
        ] {
            assert_eq!(e.remedy(), open(e), "{e:?}");
        }
        let cause = PoisonCause::MediaChanged;
        for (e, r) in [
            (ReserveError::Uncertain(cause), Remedy::ReopenStore),
            (ReserveError::Poisoned(cause), Remedy::ReopenStore),
            (ReserveError::Exhausted, Remedy::Provision),
            (ReserveError::CommitRequired, Remedy::CommitWhenSafe),
        ] {
            assert_eq!(e.remedy(), r, "{e:?}");
        }
    }
}
