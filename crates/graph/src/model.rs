//! The deployment manifest model.

use std::path::Path;

use serde::Deserialize;

use crate::error::GraphError;

/// The `apiVersion` accepted for deployment manifests.
pub const SUPPORTED_API_VERSION: &str = "deploy.neuradix.io/v1alpha1";

/// The execution class of a component (mirrors `neuradix-runtime`'s spellings;
/// see RFC-0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionClass {
    /// Externally validated bounded deadline.
    HardRealTime,
    /// Bounded queues and controlled scheduling.
    Deterministic,
    /// Responsive soft deadlines.
    Interactive,
    /// No control-path guarantee.
    BestEffort,
    /// Throughput / accelerator oriented.
    BatchAi,
}

impl ExecutionClass {
    /// Parse the canonical spelling.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "hard-real-time" => Self::HardRealTime,
            "deterministic" => Self::Deterministic,
            "interactive" => Self::Interactive,
            "best-effort" => Self::BestEffort,
            "batch-ai" => Self::BatchAi,
            _ => return None,
        })
    }

    /// The canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HardRealTime => "hard-real-time",
            Self::Deterministic => "deterministic",
            Self::Interactive => "interactive",
            Self::BestEffort => "best-effort",
            Self::BatchAi => "batch-ai",
        }
    }

    /// Whether this class is on the deterministic control path (Python is
    /// prohibited here — §12.4 EXEC-007, §19.4).
    pub const fn is_deterministic_path(self) -> bool {
        matches!(self, Self::HardRealTime | Self::Deterministic)
    }

    /// Every supported spelling.
    pub const ALL: &'static [&'static str] = &[
        "hard-real-time",
        "deterministic",
        "interactive",
        "best-effort",
        "batch-ai",
    ];
}

/// The runtime a component executes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// Native Rust.
    Rust,
    /// An isolated Python worker.
    Python,
}

impl Runtime {
    /// Parse the canonical spelling.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "rust" => Self::Rust,
            "python" => Self::Python,
            _ => return None,
        })
    }

    /// The canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }

    /// Every supported spelling.
    pub const ALL: &'static [&'static str] = &["rust", "python"];
}

/// The safety role of a component in the authority path (§16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// An ordinary component.
    Normal,
    /// The safety authority boundary.
    Safety,
    /// A safety-relevant actuator.
    Actuator,
}

impl Role {
    /// Parse the canonical spelling.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "normal" => Self::Normal,
            "safety" => Self::Safety,
            "actuator" => Self::Actuator,
            _ => return None,
        })
    }

    /// The canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Safety => "safety",
            Self::Actuator => "actuator",
        }
    }

    /// Every supported spelling.
    pub const ALL: &'static [&'static str] = &["normal", "safety", "actuator"];
}

/// A validated compute node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Node name.
    pub name: String,
    /// Target triple/profile (e.g. `linux-aarch64`).
    pub target: String,
}

/// A validated component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// Component name.
    pub name: String,
    /// The node it is placed on.
    pub node: String,
    /// Execution class.
    pub execution_class: ExecutionClass,
    /// Runtime.
    pub runtime: Runtime,
    /// Safety role.
    pub role: Role,
    /// Contract references this component provides (outputs).
    pub provides: Vec<String>,
    /// Contract references this component requires (inputs).
    pub requires: Vec<String>,
}

/// A validated connection between two components carrying a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// Producer component name.
    pub from: String,
    /// Consumer component name.
    pub to: String,
    /// The contract reference carried.
    pub contract: String,
}

/// A validated deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    /// Deployment name.
    pub name: String,
    /// Optional profile label (e.g. `marine-auv`).
    pub profile: Option<String>,
    /// Nodes.
    pub nodes: Vec<Node>,
    /// Components.
    pub components: Vec<Component>,
    /// Connections.
    pub connections: Vec<Connection>,
}

// ---------------------------------------------------------------------------
// Raw (unvalidated) deserialization types.
// ---------------------------------------------------------------------------

/// A raw, unvalidated deployment document.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDeployment {
    /// `apiVersion`
    pub api_version: Option<String>,
    /// `kind`
    pub kind: Option<String>,
    /// `metadata`
    pub metadata: Option<RawMetadata>,
    /// `spec`
    pub spec: Option<RawSpec>,
}

/// Raw metadata.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMetadata {
    /// `metadata.name`
    pub name: Option<String>,
}

/// Raw spec.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSpec {
    /// `spec.profile`
    pub profile: Option<String>,
    /// `spec.nodes`
    #[serde(default)]
    pub nodes: Vec<RawNode>,
    /// `spec.components`
    #[serde(default)]
    pub components: Vec<RawComponent>,
    /// `spec.connections`
    #[serde(default)]
    pub connections: Vec<RawConnection>,
}

/// Raw node.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawNode {
    /// `name`
    pub name: Option<String>,
    /// `target`
    pub target: Option<String>,
}

/// Raw component.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawComponent {
    /// `name`
    pub name: Option<String>,
    /// `node`
    pub node: Option<String>,
    /// `executionClass`
    pub execution_class: Option<String>,
    /// `runtime` (defaults to `rust`)
    pub runtime: Option<String>,
    /// `role` (defaults to `normal`)
    pub role: Option<String>,
    /// `provides`
    #[serde(default)]
    pub provides: Vec<String>,
    /// `requires`
    #[serde(default)]
    pub requires: Vec<String>,
}

/// Raw connection.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawConnection {
    /// `from`
    pub from: Option<String>,
    /// `to`
    pub to: Option<String>,
    /// `contract`
    pub contract: Option<String>,
}

/// Parse a raw deployment from a YAML string.
pub fn from_yaml_str(source: &str, path: &Path) -> Result<RawDeployment, GraphError> {
    serde_yaml::from_str(source).map_err(|source| GraphError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Read and parse a raw deployment from a file.
pub fn from_file(path: &Path) -> Result<RawDeployment, GraphError> {
    let source = std::fs::read_to_string(path).map_err(|source| GraphError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    from_yaml_str(&source, path)
}
