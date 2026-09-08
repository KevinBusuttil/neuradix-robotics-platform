//! Fixed, trusted holder/capability binding for an embedded command session.
use neuradix_command_core::{CommandMeta, CommandRejection, CommandSession, ConfigError, SessionConfig};
use neuradix_time::Timestamp;

/// Exactly one provisioned binding; numeric IDs are deployment-owned, not credentials.
#[derive(Debug, Clone, Copy)]
pub struct AuthorityLease {
    pub(crate) holder: u64,
    pub(crate) capability: u64,
    pub(crate) session: CommandSession,
}
impl AuthorityLease {
    /// Provision a binding with validated trusted session configuration.
    pub const fn new(holder: u64, capability: u64, config: SessionConfig) -> Self {
        Self { holder, capability, session: CommandSession::new(config) }
    }
    /// Current trusted session configuration.
    pub fn config(&self) -> SessionConfig { self.session.config() }
    /// Provisioned holder identifier.
    pub fn holder(&self) -> u64 { self.holder }
    /// Provisioned capability identifier.
    pub fn capability(&self) -> u64 { self.capability }
    /// Whether the lease grants at runtime `now` (command freshness is separate).
    pub fn grants_at(&self, now: Timestamp) -> bool { self.session.authorize(now).is_ok() }
    pub(crate) fn validate(&self, holder: u64, capability: u64, meta: CommandMeta, now: Timestamp) -> Result<(), CommandRejection> {
        if holder != self.holder || capability != self.capability { return Err(CommandRejection::UnknownBinding); }
        self.session.validate(meta, now)
    }
    pub(crate) fn replace(&mut self, lease: Self) -> Result<(), ConfigError> {
        self.session.replace(lease.config())?;
        self.holder = lease.holder;
        self.capability = lease.capability;
        Ok(())
    }
}
