//! Trusted, out-of-band provisioning of a new namespace epoch (feature
//! `provisioning`). Never enabled in actuator firmware; CI checks the MCU target.

use super::{
    NamespaceEpoch, PoisonCause, RECORD_BYTES, ReservationKey, ReservationStore, Slot, StoreError,
    View, compose, rank, read_view, record::RecordFields, verified_write,
};
use crate::Generation;

/// Optional guards for [`provision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProvisionGuards {
    /// Refuse an epoch below this trusted floor.
    pub floor: Option<NamespaceEpoch>,
    /// Highest generation ever used for this binding by any earlier allocator
    /// (legacy migration). The new epoch's first value must exceed it.
    pub prior_high_water: Option<Generation>,
}

/// Successful provisioning. Nothing was issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProvisionReport {
    /// Epoch now in force for the key.
    pub epoch: NamespaceEpoch,
    /// Previous epoch of this key's best record, if any.
    pub replaced: Option<NamespaceEpoch>,
    /// Commit count written.
    pub commits: u32,
}

/// Why provisioning refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionError {
    /// A read returned `Io` or `Unavailable`; nothing was written.
    Store(StoreError),
    /// A CRC-valid slot has an unknown format; nothing was written.
    UnsupportedFormat,
    /// A record's slot tag differs from the slot read; nothing was written.
    SlotAliasing,
    /// The requested epoch is not above every epoch present; nothing was written.
    EpochNotNewer {
        /// Highest epoch found in either slot, for any key.
        highest: NamespaceEpoch,
    },
    /// The requested epoch is below the floor; nothing was written.
    BelowFloor {
        /// Configured floor.
        floor: NamespaceEpoch,
    },
    /// The epoch's first value does not exceed the prior history; nothing was written.
    PriorHistoryNotExceeded {
        /// Configured prior high-water generation.
        prior: Generation,
    },
    /// A write or read-back failed; the epoch is burned and must not be reused.
    Uncertain(PoisonCause),
}
impl core::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for ProvisionError {}

/// TRUSTED PROVISIONING ONLY: factory, wired maintenance or a host provisioning
/// tool, over a channel separate from actuator command ingress, while no grant is
/// active, with `epoch` taken from a durable, fleet-wide, strictly increasing,
/// single-use registry. MUST NOT be called by the boot path in reaction to an
/// `open` or `reserve` failure.
///
/// Refusals cost exactly 2 reads and 0 writes. On success it writes
/// `{epoch, high_water: 0}` with verified writes, first to a slot holding
/// nothing worth keeping (not this key's best record, nor, when the key has no
/// record, a lone foreign record), then to the other. It never erases, so an interrupted
/// provision leaves the old state or the new one. A write or verify failure burns
/// the epoch. Nothing is issued: the next `open` + `reserve` commits before any
/// value is returned.
pub fn provision<S: ReservationStore + ?Sized>(
    store: &mut S,
    key: ReservationKey,
    epoch: NamespaceEpoch,
    guards: ProvisionGuards,
) -> Result<ProvisionReport, ProvisionError> {
    let mut buf = [0u8; RECORD_BYTES];
    let views = [
        read_view(store, Slot::A, key, &mut buf),
        read_view(store, Slot::B, key, &mut buf),
    ];
    for view in &views {
        match view {
            View::Unreadable => return Err(ProvisionError::Store(StoreError::Io)),
            View::Unavailable => return Err(ProvisionError::Store(StoreError::Unavailable)),
            _ => {}
        }
    }
    if views.iter().any(|v| matches!(v, View::Unsupported)) {
        return Err(ProvisionError::UnsupportedFormat);
    }
    if views.iter().any(|v| matches!(v, View::Aliased)) {
        return Err(ProvisionError::SlotAliasing);
    }
    let highest = views
        .iter()
        .filter_map(|v| match v {
            View::Mine(f) | View::Foreign(f) => Some(f.epoch),
            _ => None,
        })
        .max();
    if let Some(highest) = highest
        && highest >= epoch
    {
        return Err(ProvisionError::EpochNotNewer { highest });
    }
    if let Some(floor) = guards.floor
        && epoch < floor
    {
        return Err(ProvisionError::BelowFloor { floor });
    }
    if let Some(prior) = guards.prior_high_water
        && compose(epoch, 1) <= prior.get()
    {
        return Err(ProvisionError::PriorHistoryNotExceeded { prior });
    }
    let mine = views.map(|v| match v {
        View::Mine(f) => Some(f),
        _ => None,
    });
    let best = mine.iter().flatten().max_by_key(|f| rank(f)).copied();
    let commits = mine
        .iter()
        .flatten()
        .map(|f| f.commits)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    // Write first a slot that holds nothing worth keeping, so an interrupted
    // provision leaves the old state or the new one: this key's best record, or,
    // when the key has none, a valid foreign record (the only old state).
    let keeps = |slot: Slot| match (views[slot.index()], best) {
        (View::Mine(f), Some(b)) => rank(&f) == rank(&b),
        (View::Foreign(_), None) => true,
        _ => false,
    };
    let first = match (keeps(Slot::A), keeps(Slot::B)) {
        (true, false) => Slot::B,
        _ => Slot::A,
    };
    for slot in [first, first.other()] {
        let fields = RecordFields {
            slot,
            key,
            epoch,
            high_water: 0,
            commits,
        };
        verified_write(store, &fields, &mut buf).map_err(ProvisionError::Uncertain)?;
    }
    Ok(ProvisionReport {
        epoch,
        replaced: best.map(|b| b.epoch),
        commits,
    })
}
