//! Command authority: identities, capabilities and time-bounded leases (§16.3).

use neuradix_time::Timestamp;
use neuradix_command_core::{CommandMeta, CommandRejection, CommandSession, ConfigError, SessionConfig};

use crate::error::SafetyError;

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
///
/// Bounds can only be set through validated construction.
/// ```compile_fail
/// use neuradix_safety::CommandEnvelope;
/// let envelope = CommandEnvelope { min: f64::NAN, max: 1.0 };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandEnvelope {
    min: f64,
    max: f64,
}

impl CommandEnvelope {
    /// Construct finite, inclusive bounds with `min <= max`.
    pub fn new(min: f64, max: f64) -> Result<Self, SafetyError> {
        if !min.is_finite() || !max.is_finite() || min > max {
            return Err(SafetyError::InvalidEnvelope);
        }
        Ok(Self { min, max })
    }

    /// Minimum permitted command value.
    pub fn min(&self) -> f64 {
        self.min
    }

    /// Maximum permitted command value.
    pub fn max(&self) -> f64 {
        self.max
    }

    /// Whether `value` is within the envelope.
    pub fn permits(&self, value: f64) -> bool {
        value.is_finite() && value >= self.min && value <= self.max
    }
}

/// One trusted holder/capability binding and its current lease/session.
/// Configuration is immutable except through trusted table renewal/replacement.
#[derive(Debug, Clone)]
pub struct AuthorityLease {
    holder: Identity,
    capability: Capability,
    session: CommandSession,
    envelope: Option<CommandEnvelope>,
}

impl AuthorityLease {
    /// Provision one binding with validated session configuration.
    pub fn new(holder: Identity, capability: Capability, config: SessionConfig, envelope: Option<CommandEnvelope>) -> Self {
        Self { holder, capability, session: CommandSession::new(config), envelope }
    }
    /// The trusted holder identity.
    pub fn holder(&self) -> &Identity { &self.holder }
    /// The trusted capability identity.
    pub fn capability(&self) -> &Capability { &self.capability }
    /// Current lease/session configuration.
    pub fn config(&self) -> SessionConfig { self.session.config() }
    /// Whether the lease currently grants authority (independent of command validity).
    pub fn is_valid_at(&self, now: Timestamp) -> bool { self.session.authorize(now).is_ok() }
    fn matches(&self, holder: &Identity, capability: &Capability) -> bool {
        &self.holder == holder && &self.capability == capability
    }
}

/// Authority failures use the shared gate rejection vocabulary.
pub type AuthorityDenial = CommandRejection;

/// Maximum number of trusted bindings, including revoked tombstones.
pub const MAX_LEASE_BINDINGS: usize = 32;

/// A bounded, trusted-only binding table. Commands never allocate entries.
///
/// One current session exists per holder/capability. Revocation retains its
/// generation watermark; replacing a binding requires a greater generation.
#[derive(Debug, Clone, Default)]
pub struct LeaseTable {
    leases: Vec<AuthorityLease>,
}

impl LeaseTable {
    /// An empty table (at most [`MAX_LEASE_BINDINGS`] trusted slots).
    pub fn new() -> Self { Self::default() }
    /// Number of reserved binding slots, including revoked bindings.
    pub fn binding_count(&self) -> usize { self.leases.len() }
    /// Trusted provisioning. Replaces an existing binding only with a greater
    /// generation; use [`Self::renew`] to extend the same generation's lease.
    pub fn grant(&mut self, lease: AuthorityLease) -> Result<(), ConfigError> {
        if let Some(index) = self.index(&lease.holder, &lease.capability) {
            let old = &mut self.leases[index];
            old.session.replace(lease.config())?;
            old.envelope = lease.envelope;
        } else {
            if self.leases.len() == MAX_LEASE_BINDINGS { return Err(ConfigError::CapacityExceeded); }
            self.leases.push(lease);
        }
        Ok(())
    }
    /// Trusted renewal preserves generation, sequence, age, deadline and watchdog.
    pub fn renew(&mut self, holder: &Identity, capability: &Capability, expires: Timestamp, now: Timestamp) -> Result<(), ConfigError> {
        let index = self.index(holder, capability).ok_or(ConfigError::UnknownBinding)?;
        self.leases[index].session.renew(expires, now)
    }
    /// Revoke authority while retaining the generation watermark.
    pub fn revoke(&mut self, holder: &Identity, capability: &Capability) {
        if let Some(index) = self.index(holder, capability) { self.leases[index].session.revoke(); }
    }
    /// Last fully accepted command time for diagnostics; never receiver arrival
    /// time of rejected traffic and never proof of source freshness.
    pub fn last_accepted_at(&self, holder: &Identity, capability: &Capability) -> Option<Timestamp> {
        self.index(holder, capability).and_then(|i| self.leases[i].session.last_accepted_at())
    }
    fn index(&self, holder: &Identity, capability: &Capability) -> Option<usize> {
        self.leases.iter().position(|lease| lease.matches(holder, capability))
    }
    pub(crate) fn validate(&self, holder: &Identity, capability: &Capability, meta: CommandMeta, now: Timestamp, value: f64) -> Result<usize, CommandRejection> {
        let index = self.index(holder, capability).ok_or(CommandRejection::UnknownBinding)?;
        let lease = &self.leases[index];
        lease.session.validate(meta, now)?;
        if !value.is_finite() { return Err(CommandRejection::NonFiniteCommand); }
        if lease.envelope.is_some_and(|envelope| !envelope.permits(value)) { return Err(CommandRejection::OutOfEnvelope); }
        Ok(index)
    }
    pub(crate) fn accept(&mut self, index: usize, meta: CommandMeta, now: Timestamp) -> Result<(), CommandRejection> {
        self.leases[index].session.accept(meta, now)
    }
    pub(crate) fn check_held(&self, index: usize, generation: neuradix_command_core::Generation, now: Timestamp) -> Result<(), CommandRejection> {
        let session = &self.leases[index].session;
        if session.config().generation() != generation { return Err(CommandRejection::GenerationMismatch); }
        session.check_held(now)
    }
}
