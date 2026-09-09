//! Validated Linux per-process resource policy and bounded setup diagnostics.

use crate::WorkerError;

/// Stage reported by the trusted launcher; no worker text selects a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceStage {
    /// Launcher arguments or policy validation.
    Configuration,
    /// Platform page size or native limit representation.
    Platform,
    /// Privileged credentials/capabilities, or unavailable credential evidence.
    Privileges,
    /// Preventing privilege gains across subsequent exec.
    NoNewPrivileges,
    /// Installing both soft and hard CPU limits.
    Cpu,
    /// Installing both soft and hard address-space limits.
    AddressSpace,
    /// Reading back the installed kernel policy.
    Verification,
    /// Missing or malformed launcher confirmation.
    Protocol,
}
impl ResourceStage {
    /// Stable bounded spelling used by the launcher setup protocol.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "config",
            Self::Platform => "platform",
            Self::Privileges => "privs",
            Self::NoNewPrivileges => "nnp",
            Self::Cpu => "cpu",
            Self::AddressSpace => "as",
            Self::Verification => "verify",
            Self::Protocol => "protocol",
        }
    }
    pub(crate) fn from_str(value: &str) -> Option<Self> {
        [Self::Configuration, Self::Platform, Self::Privileges,
            Self::NoNewPrivileges, Self::Cpu, Self::AddressSpace,
            Self::Verification, Self::Protocol]
            .into_iter().find(|stage| stage.as_str() == value)
    }
}

/// Immutable per-process lifetime CPU-time and virtual-address-space limits.
/// These are not wall-clock, RSS, aggregate process-tree or GPU quotas.
/// Soft and hard kernel values are equal; a worker cannot raise its hard limit.
///
/// ```compile_fail
/// use neuradix_python::ResourceLimits;
/// let mut limits = ResourceLimits::default();
/// limits.cpu_seconds = 0;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    cpu_seconds: u64,
    address_space_bytes: u64,
}
impl ResourceLimits {
    /// Validate integral CPU seconds (1..=86,400) and virtual address bytes
    /// (16 MiB..=1 TiB). Zero, infinity and out-of-range values are rejected.
    pub fn new(cpu_seconds: u64, address_space_bytes: u64) -> Result<Self, WorkerError> {
        if !(1..=86_400).contains(&cpu_seconds)
            || !(16_777_216..=1_099_511_627_776).contains(&address_space_bytes)
        {
            return Err(WorkerError::InvalidConfig("invalid CPU/address-space limits"));
        }
        Ok(Self { cpu_seconds, address_space_bytes })
    }
    /// Integral CPU seconds across all threads for the lifetime of this process.
    pub fn cpu_seconds(self) -> u64 { self.cpu_seconds }
    /// Virtual address-space bytes. Resolved policy is rounded down to pages.
    pub fn address_space_bytes(self) -> u64 { self.address_space_bytes }

    /// Resolve page rounding and native representation without installing limits.
    /// This method never changes the calling process's resource limits.
    /// Successful resolution is not evidence that a worker was constrained.
    pub fn for_current_platform(self) -> Result<Self, WorkerError> {
        #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
        {
            use nix::sys::resource::RLIM_INFINITY;
            use nix::unistd::{SysconfVar, sysconf};
            let page = sysconf(SysconfVar::PAGE_SIZE)
                .map_err(|error| WorkerError::ResourceSetup { stage: ResourceStage::Platform, errno: Some(error as i32) })?
                .and_then(|value| u64::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or(WorkerError::ResourceSetup { stage: ResourceStage::Platform, errno: None })?;
            let bytes = self.address_space_bytes / page * page;
            if u128::from(bytes) >= u128::from(RLIM_INFINITY)
                || u128::from(self.cpu_seconds) >= u128::from(RLIM_INFINITY)
            {
                return Err(WorkerError::ResourceSetup { stage: ResourceStage::Platform, errno: None });
            }
            Self::new(self.cpu_seconds, bytes)
        }
        #[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
        {
            Err(WorkerError::UnsupportedPlatform)
        }
    }
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self::new(300, 268_435_456).expect("valid resource defaults")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_policy_boundaries_reject_invalid_or_unlimited_values() {
        for cpu in [0, 86_401, u64::MAX] {
            assert!(ResourceLimits::new(cpu, 268_435_456).is_err());
        }
        for bytes in [0, 16_777_215, 1_099_511_627_777, u64::MAX] {
            assert!(ResourceLimits::new(300, bytes).is_err());
        }
        assert!(ResourceLimits::new(1, 16_777_216).is_ok());
        assert!(ResourceLimits::new(86_400, 1_099_511_627_776).is_ok());
    }
    #[test]
    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    fn page_rounding_is_inward_and_does_not_change_supervisor_limits() {
        use nix::sys::resource::{Resource, getrlimit};
        let before = (getrlimit(Resource::RLIMIT_CPU).unwrap(), getrlimit(Resource::RLIMIT_AS).unwrap());
        let policy = ResourceLimits::new(1, 268_435_457).unwrap();
        assert_eq!(policy.for_current_platform().unwrap().address_space_bytes(), 268_435_456);
        assert_eq!(before, (getrlimit(Resource::RLIMIT_CPU).unwrap(), getrlimit(Resource::RLIMIT_AS).unwrap()));
    }
}
