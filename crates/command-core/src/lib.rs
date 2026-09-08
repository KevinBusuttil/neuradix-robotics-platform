//! Shared, allocation-free command validity for host and MCU gates.
//!
//! A trusted issuer provisions a non-reused generation, lease and shared clock
//! relationship. Payloads cannot provision or reset a session. Sequence numbers
//! and generation identifiers are replay checks, **not authentication**.
//! Call gates periodically, including with no input, to enforce held-output expiry.
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use neuradix_time::{ClockDomain, Duration, Timestamp};

pub mod slew;
pub use slew::SlewRate;

/// A nonzero, monotonically allocated lease/session generation.
///
/// Trusted startup MUST durably reserve a value greater than every previous
/// generation for this receiver/binding, before enabling command ingress. A
/// reset counter or value learned from a command is not a generation allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Generation(u128);

impl Generation {
    /// Wrap a value supplied by trusted durable initialization. Zero is reserved.
    pub const fn new(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// The identifier to echo in command metadata.
    pub const fn get(self) -> u128 {
        self.0
    }
}

/// Trusted declaration of a shared reference clock and epoch.
///
/// Equal domain labels alone do not prove a shared timeline. Provision this
/// only when the source and receiver actually use the same reference/epoch.
/// Offset conversion and independent device clocks are unsupported in A04.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedTimeline {
    id: u64,
    domain: ClockDomain,
}

impl SharedTimeline {
    /// Declare a nonzero deployment-owned timeline identity.
    pub fn new(id: u64, domain: ClockDomain) -> Result<Self, ConfigError> {
        if id == 0 {
            return Err(ConfigError::InvalidTimeline);
        }
        Ok(Self { id, domain })
    }
    /// Deployment-owned identity (not a credential).
    pub const fn id(self) -> u64 {
        self.id
    }
    /// The shared clock domain.
    pub const fn domain(self) -> ClockDomain {
        self.domain
    }
}

/// Immutable freshness and accepted-command watchdog policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandPolicy {
    timeline: SharedTimeline,
    max_age: Duration,
    future_skew: Duration,
    watchdog_timeout: Duration,
}

impl CommandPolicy {
    /// Require positive age/timeout and non-negative future skew. Age and skew
    /// are inclusive; watchdog expires strictly after its timeout.
    pub fn new(
        timeline: SharedTimeline,
        max_age: Duration,
        future_skew: Duration,
        watchdog_timeout: Duration,
    ) -> Result<Self, ConfigError> {
        if max_age.as_nanos() <= 0 || future_skew.as_nanos() < 0 || watchdog_timeout.as_nanos() <= 0
        {
            return Err(ConfigError::InvalidDurations);
        }
        Ok(Self {
            timeline,
            max_age,
            future_skew,
            watchdog_timeout,
        })
    }
    /// The supported source clock relationship.
    pub const fn timeline(self) -> SharedTimeline {
        self.timeline
    }
    /// Maximum inclusive source age.
    pub const fn max_age(self) -> Duration {
        self.max_age
    }
    /// Maximum inclusive future displacement.
    pub const fn future_skew(self) -> Duration {
        self.future_skew
    }
    /// Maximum inclusive gap since a fully accepted command.
    pub const fn watchdog_timeout(self) -> Duration {
        self.watchdog_timeout
    }
}

/// Source-provided validity metadata. It never supplies evaluation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandMeta {
    /// Echo of the generation granted by the trusted issuer.
    pub generation: Generation,
    /// Strictly increasing within a generation; gaps allowed, wrap forbidden.
    pub sequence: u64,
    /// Original source timestamp, never substituted with receiver arrival time.
    pub source_at: Timestamp,
    /// Exclusive command deadline in the source timeline.
    pub deadline: Timestamp,
    /// Echo of the provisioned shared-timeline identity.
    pub timeline: u64,
}

/// Immutable trusted lease/session configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    generation: Generation,
    issued: Timestamp,
    expires: Timestamp,
    policy: CommandPolicy,
}

impl SessionConfig {
    /// Validate `[issued, expires)` on the policy's shared timeline.
    pub fn new(
        generation: Generation,
        issued: Timestamp,
        expires: Timestamp,
        policy: CommandPolicy,
    ) -> Result<Self, ConfigError> {
        if issued.domain() != policy.timeline.domain
            || expires.domain() != issued.domain()
            || issued.as_nanos() >= expires.as_nanos()
        {
            return Err(ConfigError::InvalidLease);
        }
        Ok(Self {
            generation,
            issued,
            expires,
            policy,
        })
    }
    /// Current trusted generation.
    pub const fn generation(self) -> Generation {
        self.generation
    }
    /// Lease issue time, inclusive.
    pub const fn issued(self) -> Timestamp {
        self.issued
    }
    /// Lease expiry, exclusive.
    pub const fn expires(self) -> Timestamp {
        self.expires
    }
    /// Validated policy.
    pub const fn policy(self) -> CommandPolicy {
        self.policy
    }
}

/// Trusted configuration failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// Shared timeline identity zero is reserved.
    InvalidTimeline,
    /// Age/timeout must be positive; future skew must be non-negative.
    InvalidDurations,
    /// Lease bounds or clock domain are invalid.
    InvalidLease,
    /// A replacement generation must be strictly greater, never reused.
    ReusedGeneration,
    /// A revoked session requires a new generation, not renewal.
    RevokedSession,
    /// Same-generation renewal must occur while the existing lease is valid.
    InactiveLease,
    /// Renewal time predates runtime observation, differs in domain, or the clock faulted.
    InvalidEvaluationTime,
    /// Trusted host binding capacity was exhausted.
    CapacityExceeded,
    /// Trusted operation named a binding that is not provisioned.
    UnknownBinding,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for ConfigError {}

/// Stable, shared rejection vocabulary for both command gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRejection {
    /// Runtime evaluation clock changed domain; latched.
    EvaluationClockMismatch,
    /// Runtime time moved backwards; latched.
    EvaluationTimeRegression,
    /// Runtime elapsed subtraction overflowed; latched.
    EvaluationTimeOverflow,
    /// Holder/capability is not provisioned at this receiver.
    UnknownBinding,
    /// Authority was explicitly revoked.
    LeaseRevoked,
    /// Lease issue time has not arrived.
    LeaseNotYetValid,
    /// Lease expiry is reached (exclusive upper boundary).
    LeaseExpired,
    /// Payload generation differs from the current trusted generation.
    GenerationMismatch,
    /// Timeline identity or source/deadline domain is unsupported.
    UnsupportedClockRelationship,
    /// Deadline must be later than the source timestamp.
    InvalidDeadline,
    /// Command deadline has been reached.
    DeadlineExpired,
    /// Source is older than the inclusive maximum age.
    StaleCommand,
    /// Source is farther into the future than permitted skew.
    FutureCommand,
    /// Source/evaluation or watchdog subtraction cannot be represented.
    CommandTimeOverflow,
    /// Sequence equals the last accepted sequence.
    Duplicate,
    /// Sequence is smaller than the last accepted sequence.
    OutOfOrder,
    /// Sequence MAX was already accepted; a new generation is required.
    SequenceExhausted,
    /// No command has been accepted in this session.
    NoCommand,
    /// Gap since last accepted command exceeds the watchdog timeout.
    WatchdogExpired,
    /// Requested value is NaN or infinite.
    NonFiniteCommand,
    /// Requested value violates the authority envelope.
    OutOfEnvelope,
    /// Constraint arithmetic or final hard output validity failed.
    InvalidOutput,
}
impl core::fmt::Display for CommandRejection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Gate-wide evaluation time state; trusted session updates never reset it.
#[derive(Debug, Clone, Copy, Default)]
pub struct EvaluationClock {
    last: Option<Timestamp>,
    fault: Option<CommandRejection>,
}
impl EvaluationClock {
    /// Check a trusted control-plane timestamp against the gate's runtime clock.
    /// Does not advance evaluation time or clear a latched fault.
    pub fn check_control_time(&self, now: Timestamp) -> Result<(), ConfigError> {
        if self.fault.is_some()
            || self.last.is_some_and(|last| {
                now.domain() != last.domain()
                    || now.as_nanos() < last.as_nanos()
                    || now.duration_since(last).is_err()
            })
        {
            return Err(ConfigError::InvalidEvaluationTime);
        }
        Ok(())
    }

    /// Observe runtime time even for rejected commands and idle ticks. Return
    /// elapsed time from the previous evaluation, or None for the first tick.
    pub fn observe(&mut self, now: Timestamp) -> Result<Option<Duration>, CommandRejection> {
        if let Some(reason) = self.fault {
            return Err(reason);
        }
        let dt = if let Some(last) = self.last {
            let reason = if now.domain() != last.domain() {
                Some(CommandRejection::EvaluationClockMismatch)
            } else if now.as_nanos() < last.as_nanos() {
                Some(CommandRejection::EvaluationTimeRegression)
            } else {
                None
            };
            if let Some(reason) = reason {
                self.fault = Some(reason);
                return Err(reason);
            }
            match now.duration_since(last) {
                Ok(dt) => Some(dt),
                Err(_) => {
                    self.fault = Some(CommandRejection::EvaluationTimeOverflow);
                    return Err(CommandRejection::EvaluationTimeOverflow);
                }
            }
        } else {
            None
        };
        self.last = Some(now);
        Ok(dt)
    }
}

/// One bounded tracking slot, allocated only by trusted configuration.
#[derive(Debug, Clone, Copy)]
pub struct CommandSession {
    config: SessionConfig,
    revoked: bool,
    accepted: Option<(CommandMeta, Timestamp)>,
}
impl CommandSession {
    /// Start without any accepted command. Restart generation non-reuse is the
    /// trusted caller's durable initialization obligation (see [`Generation`]).
    pub const fn new(config: SessionConfig) -> Self {
        Self {
            config,
            revoked: false,
            accepted: None,
        }
    }
    /// Read-only trusted configuration.
    pub const fn config(&self) -> SessionConfig {
        self.config
    }
    /// Last fully accepted command's receiver time; rejected traffic cannot set it.
    pub fn last_accepted_at(&self) -> Option<Timestamp> {
        self.accepted.map(|(_, at)| at)
    }
    /// Last fully accepted command sequence.
    pub fn last_sequence(&self) -> Option<u64> {
        self.accepted.map(|(meta, _)| meta.sequence)
    }
    /// Revoke while retaining the generation and sequence watermark.
    pub fn revoke(&mut self) {
        self.revoked = true;
    }
    /// Extend only the lease expiry; preserve sequence, source age, deadline and watchdog.
    pub fn renew(
        &mut self,
        expires: Timestamp,
        now: Timestamp,
        clock: &EvaluationClock,
    ) -> Result<(), ConfigError> {
        clock.check_control_time(now)?;
        if self.revoked {
            return Err(ConfigError::RevokedSession);
        }
        if self.authorize(now).is_err() {
            return Err(ConfigError::InactiveLease);
        }
        if expires.domain() != self.config.expires.domain()
            || expires.as_nanos() <= self.config.expires.as_nanos()
        {
            return Err(ConfigError::InvalidLease);
        }
        self.config.expires = expires;
        Ok(())
    }
    /// Trusted replacement of a generation. Payloads must never call this path.
    /// Same/older generations are rejected, including after revocation.
    pub fn replace(&mut self, config: SessionConfig) -> Result<(), ConfigError> {
        if config.generation <= self.config.generation {
            return Err(ConfigError::ReusedGeneration);
        }
        *self = Self::new(config);
        Ok(())
    }
    /// Check current lease authorization using runtime time.
    pub fn authorize(&self, now: Timestamp) -> Result<(), CommandRejection> {
        if self.revoked {
            return Err(CommandRejection::LeaseRevoked);
        }
        if now.domain() != self.config.issued.domain() {
            return Err(CommandRejection::UnsupportedClockRelationship);
        }
        if now.as_nanos() < self.config.issued.as_nanos() {
            return Err(CommandRejection::LeaseNotYetValid);
        }
        if now.as_nanos() >= self.config.expires.as_nanos() {
            return Err(CommandRejection::LeaseExpired);
        }
        Ok(())
    }
    fn time_valid(&self, meta: CommandMeta, now: Timestamp) -> Result<(), CommandRejection> {
        if meta.generation != self.config.generation {
            return Err(CommandRejection::GenerationMismatch);
        }
        let timeline = self.config.policy.timeline;
        if meta.timeline != timeline.id
            || meta.source_at.domain() != timeline.domain
            || meta.deadline.domain() != timeline.domain
        {
            return Err(CommandRejection::UnsupportedClockRelationship);
        }
        if meta.deadline.as_nanos() <= meta.source_at.as_nanos() {
            return Err(CommandRejection::InvalidDeadline);
        }
        if now.as_nanos() >= meta.deadline.as_nanos() {
            return Err(CommandRejection::DeadlineExpired);
        }
        // Compare signed values without abs/negation at i128::MIN.
        let age = now
            .duration_since(meta.source_at)
            .map_err(|_| CommandRejection::CommandTimeOverflow)?
            .as_nanos();
        if age > self.config.policy.max_age.as_nanos() {
            return Err(CommandRejection::StaleCommand);
        }
        if age < -self.config.policy.future_skew.as_nanos() {
            return Err(CommandRejection::FutureCommand);
        }
        Ok(())
    }
    /// Pure validation; it does not feed the watchdog or consume a sequence.
    pub fn validate(&self, meta: CommandMeta, now: Timestamp) -> Result<(), CommandRejection> {
        self.authorize(now)?;
        self.time_valid(meta, now)?;
        if let Some(last) = self.last_sequence() {
            if last == u64::MAX {
                return Err(CommandRejection::SequenceExhausted);
            }
            if meta.sequence == last {
                return Err(CommandRejection::Duplicate);
            }
            if meta.sequence < last {
                return Err(CommandRejection::OutOfOrder);
            }
        }
        Ok(())
    }
    /// Commit only after the gate's numeric and output checks have passed.
    /// Revalidation prevents unchecked state construction through this API.
    pub fn accept(&mut self, meta: CommandMeta, now: Timestamp) -> Result<(), CommandRejection> {
        self.validate(meta, now)?;
        self.accepted = Some((meta, now));
        Ok(())
    }
    /// Check the accepted command at an idle evaluation tick, without feeding
    /// anything. Lease, source age, deadline and accepted-command watchdog all apply.
    pub fn check_held(&self, now: Timestamp) -> Result<(), CommandRejection> {
        self.authorize(now)?;
        let (meta, accepted_at) = self.accepted.ok_or(CommandRejection::NoCommand)?;
        self.time_valid(meta, now)?;
        let elapsed = now
            .duration_since(accepted_at)
            .map_err(|_| CommandRejection::CommandTimeOverflow)?
            .as_nanos();
        if elapsed < 0 {
            return Err(CommandRejection::EvaluationTimeRegression);
        }
        if elapsed > self.config.policy.watchdog_timeout.as_nanos() {
            return Err(CommandRejection::WatchdogExpired);
        }
        Ok(())
    }
}
