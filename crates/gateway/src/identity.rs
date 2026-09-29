//! Identity reporting (NRX-EMB-005 inputs), with the source of each value.
//!
//! The gateway reports the full manifest digest it verified together with the
//! node, firmware/build and deployment identities it was configured with. Each
//! value records where it came from. None of them is evidence of what a
//! connected board is running: there is no link session, manifest exchange or
//! firmware attestation in this increment (WP-B06).

use std::fmt;

use neuradix_embedded_core::DeploymentId;

/// Where a reported identity came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySource {
    /// Read from a configuration artifact supplied to this gateway instance.
    ConfiguredArtifact,
    /// Compiled into this build by a generator or the build itself.
    GeneratedBuild,
    /// A fixed value of a simulated producer or test fixture.
    SimulatedFixture,
}

impl IdentitySource {
    /// Stable label for reports.
    pub fn label(self) -> &'static str {
        match self {
            IdentitySource::ConfiguredArtifact => "configured artifact",
            IdentitySource::GeneratedBuild => "generated build information",
            IdentitySource::SimulatedFixture => "simulated fixture",
        }
    }
}

/// A value and its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributed<T> {
    /// The value.
    pub value: T,
    /// Where it came from.
    pub source: IdentitySource,
}

impl<T> Attributed<T> {
    /// Attribute `value` to `source`.
    pub fn new(value: T, source: IdentitySource) -> Self {
        Self { value, source }
    }
}

/// Identities the gateway is configured with. Owned strings, so host
/// configuration needs no `'static` lifetimes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityClaims {
    /// Node the producer is expected to be. `neuradix_embedded_core::NodeId`
    /// is not used because it requires a `'static` name.
    pub node: Option<Attributed<String>>,
    /// Producer firmware or build identity.
    pub firmware: Option<Attributed<String>>,
    /// Topology/deployment identity.
    pub deployment: Option<Attributed<DeploymentId>>,
}

/// The identity report for one gateway instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityReport {
    /// Full manifest digest, `sha256:<64 hex>`. Its source is the configured
    /// manifest artifact, and initialization verified that the compiled
    /// generated board table has the same digest and entries.
    pub manifest_digest: String,
    /// This gateway's build (crate name and version).
    pub gateway_build: Attributed<String>,
    /// Configured identities, as supplied.
    pub claims: IdentityClaims,
}

impl IdentityReport {
    /// What the report does not establish.
    pub const LIMITATION: &'static str = "no peer attestation: identities are configured, \
        generated or simulated, not reported by a connected board (link sessions are WP-B06)";
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl fmt::Display for IdentityReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  manifest digest  {}  [{}; equals the compiled board table ({})]",
            self.manifest_digest,
            IdentitySource::ConfiguredArtifact.label(),
            IdentitySource::GeneratedBuild.label()
        )?;
        let absent = "(not configured)";
        match &self.claims.node {
            Some(a) => writeln!(f, "  node             {}  [{}]", a.value, a.source.label())?,
            None => writeln!(f, "  node             {absent}")?,
        }
        match &self.claims.firmware {
            Some(a) => writeln!(f, "  firmware/build   {}  [{}]", a.value, a.source.label())?,
            None => writeln!(f, "  firmware/build   {absent}")?,
        }
        match &self.claims.deployment {
            Some(a) => writeln!(
                f,
                "  deployment       sha256:{}  [{}]",
                hex(a.value.as_bytes()),
                a.source.label()
            )?,
            None => writeln!(f, "  deployment       {absent}")?,
        }
        writeln!(
            f,
            "  gateway build    {}  [{}]",
            self.gateway_build.value,
            self.gateway_build.source.label()
        )?;
        write!(f, "  limitation       {}", Self::LIMITATION)
    }
}
