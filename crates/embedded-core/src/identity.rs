//! Node and deployment identity, provisioned into firmware.

/// A stable node identity.
///
/// Constrained targets have no heap, so the name is a `&'static str` baked into
/// the firmware image rather than an owned string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeId(&'static str);

impl NodeId {
    /// Construct a node identity from a static name.
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The node name.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// A content-addressed deployment identity (32 bytes, e.g. a SHA-256) provisioned
/// into the firmware so a node can report which deployment it belongs to (§28.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentId([u8; 32]);

impl DeploymentId {
    /// Construct a deployment identity from its 32 bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw 32 identity bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
