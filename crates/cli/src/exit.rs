//! Stable process exit codes for the `neuradix` CLI.
//!
//! The full set is defined by `docs/Neuradix_CLI_Command_Specification_v0.1.md`
//! and `docs/rfcs/RFC-0013-CLI-Command-Output-and-Automation-Contract.md`. Every
//! documented code is represented here even where this increment does not yet
//! produce all of them, so the automation contract is complete and stable.

/// A documented CLI exit code.
///
/// The complete documented set is represented deliberately, even though this
/// increment only produces a subset (`Success`, `GeneralFailure`, `InvalidUse`
/// and `ContractValidation`). `dead_code` is allowed so the automation contract
/// stays complete and stable as later commands begin using the remaining codes.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    /// The command completed successfully.
    Success = 0,
    /// An unclassified failure occurred.
    GeneralFailure = 1,
    /// The command line itself was invalid.
    InvalidUse = 2,
    /// A contract failed validation.
    ContractValidation = 3,
    /// A compatibility check failed.
    Compatibility = 4,
    /// A required connection could not be established.
    Connectivity = 5,
    /// Authentication failed.
    Authentication = 6,
    /// Authorization was denied.
    Authorization = 7,
    /// An operation was rejected by Safety.
    SafetyRejection = 8,
    /// A determinism / replay mismatch was detected.
    DeterminismMismatch = 9,
    /// A deployment failed validation.
    DeploymentValidation = 10,
    /// The operation completed only partially.
    PartialOperation = 11,
    /// The operation timed out.
    Timeout = 12,
}

impl ExitCode {
    /// The numeric process exit code.
    pub const fn code(self) -> i32 {
        self as i32
    }
}
