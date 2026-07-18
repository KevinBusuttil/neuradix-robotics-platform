//! The component definition and manifest.

use neuradix_contracts::SchemaId;
use semver::Version;

use crate::error::ComponentError;
use crate::health::HealthState;
use crate::id::ComponentId;

/// The execution class of a component (§12.1).
///
/// Only the classification is modelled in this increment; the deterministic and
/// asynchronous executors are a later increment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionClass {
    /// Externally validated bounded deadline (e.g. MCU motor control).
    HardRealTime,
    /// Bounded queues and controlled scheduling (control, estimation).
    Deterministic,
    /// Responsive soft deadlines (drivers, mission logic).
    Interactive,
    /// No control-path guarantee (UI, logging, maintenance).
    BestEffort,
    /// Throughput / accelerator oriented (inference, mapping).
    BatchAi,
}

impl ExecutionClass {
    /// The canonical lowercase spelling of the class.
    pub const fn as_str(self) -> &'static str {
        match self {
            ExecutionClass::HardRealTime => "hard-real-time",
            ExecutionClass::Deterministic => "deterministic",
            ExecutionClass::Interactive => "interactive",
            ExecutionClass::BestEffort => "best-effort",
            ExecutionClass::BatchAi => "batch-ai",
        }
    }
}

impl std::fmt::Display for ExecutionClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A component's declared identity and interface surface.
///
/// This is a minimal manifest for the foundation increment: enough to identify
/// a component, its version, its execution class and the contracts it provides
/// and requires (by content-addressed schema identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentManifest {
    /// Stable logical identity.
    pub id: ComponentId,
    /// Human-readable component name.
    pub name: String,
    /// Component version.
    pub version: Version,
    /// Execution class.
    pub execution_class: ExecutionClass,
    /// Schema identities of contracts this component provides (outputs).
    pub provides: Vec<SchemaId>,
    /// Schema identities of contracts this component requires (inputs).
    pub requires: Vec<SchemaId>,
}

impl ComponentManifest {
    /// Construct a manifest with no declared ports.
    pub fn new(id: ComponentId, name: impl Into<String>, execution_class: ExecutionClass) -> Self {
        Self {
            id,
            name: name.into(),
            version: Version::new(0, 0, 0),
            execution_class,
            provides: Vec::new(),
            requires: Vec::new(),
        }
    }
}

/// A managed Neuradix component.
///
/// Lifecycle hooks default to no-ops so simple components only implement what
/// they need. Actuator-affecting behaviour must not begin before a component is
/// [`crate::LifecycleState::Active`] and has received authority (§8.2); that
/// authority path is enforced by `neuradix-safety` in a later increment.
pub trait Component {
    /// The component's stable identity.
    fn id(&self) -> &ComponentId;

    /// Validate and apply configuration (`Declared -> Configured`).
    fn on_configure(&mut self) -> Result<(), ComponentError> {
        Ok(())
    }

    /// Begin processing (`Inactive -> Active`).
    fn on_activate(&mut self) -> Result<(), ComponentError> {
        Ok(())
    }

    /// Stop processing but remain resolved (`Active -> Inactive` paths).
    fn on_deactivate(&mut self) -> Result<(), ComponentError> {
        Ok(())
    }

    /// Release resources during shutdown (`Stopping -> Stopped`).
    fn on_stop(&mut self) -> Result<(), ComponentError> {
        Ok(())
    }

    /// The component's current health.
    fn health(&self) -> HealthState {
        HealthState::Unknown
    }
}
