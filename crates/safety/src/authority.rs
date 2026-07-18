//! Command authority: identities, capabilities and time-bounded leases (§16.3).

use std::cmp::Ordering;

use neuradix_time::Timestamp;

/// The identity of a command source (operator, planner, controller).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identity(String);

impl Identity {
    /// Construct an identity.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    /// The identity as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A controlled capability, e.g. `propulsion/vertical-thrust`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Capability(String);

impl Capability {
    /// Construct a capability name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    /// The capability as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The permitted command envelope of a lease: an inclusive value range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandEnvelope {
    /// Minimum permitted command value.
    pub min: f64,
    /// Maximum permitted command value.
    pub max: f64,
}

impl CommandEnvelope {
    /// Whether `value` is within the envelope.
    pub fn permits(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

/// A time-bounded grant of authority over a capability (§16.3).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorityLease {
    /// Who holds the authority.
    pub holder: Identity,
    /// The controlled capability.
    pub capability: Capability,
    /// Arbitration priority (higher wins).
    pub priority: u8,
    /// When the lease becomes valid.
    pub issued: Timestamp,
    /// When the lease expires (exclusive).
    pub expires: Timestamp,
    /// Optional permitted command envelope.
    pub envelope: Option<CommandEnvelope>,
}

impl AuthorityLease {
    /// Whether the lease is valid at `at` (same clock domain, `issued <= at < expires`).
    pub fn is_valid_at(&self, at: Timestamp) -> bool {
        matches!(
            self.issued.compare(at),
            Ok(Ordering::Less | Ordering::Equal)
        ) && matches!(at.compare(self.expires), Ok(Ordering::Less))
    }

    fn matches(&self, holder: &Identity, capability: &Capability) -> bool {
        &self.holder == holder && &self.capability == capability
    }
}

/// Why an authority check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthorityDenial {
    /// No lease matches the holder and capability.
    #[error("no authority lease for holder/capability")]
    NoLease,
    /// A matching lease exists but has not yet become valid.
    #[error("authority lease is not yet valid")]
    NotYetValid,
    /// A matching lease exists but has expired.
    #[error("authority lease has expired")]
    Expired,
    /// The commanded value is outside the lease's permitted envelope.
    #[error("commanded value is outside the permitted envelope")]
    OutOfEnvelope,
}

/// A table of active authority leases.
#[derive(Debug, Clone, Default)]
pub struct LeaseTable {
    leases: Vec<AuthorityLease>,
}

impl LeaseTable {
    /// An empty lease table.
    pub fn new() -> Self {
        Self { leases: Vec::new() }
    }

    /// Grant (add) a lease.
    pub fn grant(&mut self, lease: AuthorityLease) {
        self.leases.push(lease);
    }

    /// Revoke all leases matching a holder and capability.
    pub fn revoke(&mut self, holder: &Identity, capability: &Capability) {
        self.leases.retain(|l| !l.matches(holder, capability));
    }

    /// Authorize a command: returns the winning lease, or a typed denial.
    ///
    /// Among leases matching holder+capability, the highest-priority lease valid
    /// at `at` wins. If it carries an envelope, `value` must be within it.
    pub fn authorize(
        &self,
        holder: &Identity,
        capability: &Capability,
        at: Timestamp,
        value: f64,
    ) -> Result<&AuthorityLease, AuthorityDenial> {
        let matching: Vec<&AuthorityLease> = self
            .leases
            .iter()
            .filter(|l| l.matches(holder, capability))
            .collect();
        if matching.is_empty() {
            return Err(AuthorityDenial::NoLease);
        }

        let winner = matching
            .iter()
            .filter(|l| l.is_valid_at(at))
            .max_by_key(|l| l.priority)
            .copied();

        match winner {
            Some(lease) => match &lease.envelope {
                Some(envelope) if !envelope.permits(value) => Err(AuthorityDenial::OutOfEnvelope),
                _ => Ok(lease),
            },
            None => {
                // A matching lease exists but none is valid: distinguish not-yet
                // from expired using the earliest issue time.
                let not_yet = matching
                    .iter()
                    .any(|l| matches!(at.compare(l.issued), Ok(Ordering::Less)));
                if not_yet {
                    Err(AuthorityDenial::NotYetValid)
                } else {
                    Err(AuthorityDenial::Expired)
                }
            }
        }
    }
}
