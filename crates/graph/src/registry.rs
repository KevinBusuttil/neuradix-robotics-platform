//! A contract registry: the set of real, validated contracts a deployment's
//! references are resolved against (RFC-0002 ↔ RFC-0019).
//!
//! A deployment's `connections[].contract` is a *reference*, not a contract. To
//! honour "contracts before connectivity" (§3.1, §6.3) the graph compiler must
//! prove every reference resolves to a real, validated schema and pin the
//! content-addressed schema identity it resolved to. A [`ContractRegistry`]
//! indexes authored contracts by `namespace/name` and version for exactly that.
//!
//! A reference is `namespace/name` (resolved when the registry holds exactly one
//! version) or `namespace/name@major.minor.patch` (pinned to an exact version).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use neuradix_contracts::{ContractError, load_file, schema_identity};

/// A contract known to the registry: its identifier, version and the
/// content-addressed schema identity a reference to it resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractEntry {
    /// The `namespace/name` identifier.
    pub identifier: String,
    /// The semantic version.
    pub version: String,
    /// The `sha256:` schema identity.
    pub schema_id: String,
}

/// The outcome of resolving a single contract reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<'a> {
    /// The reference resolved to exactly one registered contract.
    Resolved(&'a ContractEntry),
    /// No contract with the referenced `namespace/name` is registered.
    UnknownContract,
    /// The `namespace/name` is registered but not at the pinned version.
    UnknownVersion,
    /// The `namespace/name` is registered at several versions and the reference
    /// did not pin one. Carries the available versions (sorted).
    Ambiguous(Vec<String>),
    /// The reference itself was malformed (empty, or empty version after `@`).
    Malformed,
}

/// Errors from *building* a registry (as opposed to resolving a reference,
/// which is reported as a [`Resolution`]).
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// A directory could not be read.
    #[error("failed to read contract directory `{path}`: {source}")]
    Io {
        /// Directory that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A contract file failed to load, parse or validate.
    #[error("invalid contract `{path}`: {source}")]
    Contract {
        /// The offending contract file.
        path: PathBuf,
        /// Underlying contract error.
        #[source]
        source: ContractError,
    },

    /// Two contracts share an identifier and version.
    #[error("duplicate contract `{identifier}@{version}` (`{first}` and `{second}`)")]
    Duplicate {
        /// The clashing `namespace/name`.
        identifier: String,
        /// The clashing version.
        version: String,
        /// The first file seen.
        first: PathBuf,
        /// The second file seen.
        second: PathBuf,
    },
}

/// An index of validated contracts, keyed by `namespace/name` then version.
#[derive(Debug, Default, Clone)]
pub struct ContractRegistry {
    // identifier -> (version -> entry), both ordered for deterministic output.
    by_id: BTreeMap<String, BTreeMap<String, ContractEntry>>,
    // Remember source files to report duplicates precisely.
    sources: BTreeMap<(String, String), PathBuf>,
}

impl ContractRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of registered `(identifier, version)` contracts.
    pub fn len(&self) -> usize {
        self.by_id.values().map(BTreeMap::len).sum()
    }

    /// Whether the registry holds no contracts.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Load every `*.yaml` / `*.yml` contract under `dir` (searched
    /// recursively) into a registry.
    pub fn load_dir(dir: &Path) -> Result<Self, RegistryError> {
        let mut files = Vec::new();
        collect_yaml(dir, &mut files)?;
        files.sort();

        let mut registry = Self::new();
        for file in files {
            let contract = load_file(&file).map_err(|source| RegistryError::Contract {
                path: file.clone(),
                source,
            })?;
            registry.insert(
                ContractEntry {
                    identifier: contract.identifier(),
                    version: contract.metadata.version.to_string(),
                    schema_id: schema_identity(&contract).as_str().to_owned(),
                },
                file,
            )?;
        }
        Ok(registry)
    }

    /// Insert an entry, rejecting a duplicate `(identifier, version)`.
    fn insert(&mut self, entry: ContractEntry, source: PathBuf) -> Result<(), RegistryError> {
        let key = (entry.identifier.clone(), entry.version.clone());
        if let Some(first) = self.sources.get(&key) {
            return Err(RegistryError::Duplicate {
                identifier: entry.identifier,
                version: entry.version,
                first: first.clone(),
                second: source,
            });
        }
        self.sources.insert(key, source);
        self.by_id
            .entry(entry.identifier.clone())
            .or_default()
            .insert(entry.version.clone(), entry);
        Ok(())
    }

    /// Resolve a contract reference (`namespace/name` or `namespace/name@ver`).
    pub fn resolve(&self, reference: &str) -> Resolution<'_> {
        let (identifier, version) = match parse_reference(reference) {
            Some(parts) => parts,
            None => return Resolution::Malformed,
        };

        let Some(versions) = self.by_id.get(identifier) else {
            return Resolution::UnknownContract;
        };

        match version {
            Some(v) => match versions.get(v) {
                Some(entry) => Resolution::Resolved(entry),
                None => Resolution::UnknownVersion,
            },
            None => {
                if versions.len() == 1 {
                    // Exactly one version: unambiguous.
                    Resolution::Resolved(versions.values().next().expect("one entry"))
                } else {
                    Resolution::Ambiguous(versions.keys().cloned().collect())
                }
            }
        }
    }
}

/// Split a reference into `(identifier, Some(version))` or `(identifier, None)`.
/// Returns `None` if the reference or the pinned version is empty.
fn parse_reference(reference: &str) -> Option<(&str, Option<&str>)> {
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }
    match reference.split_once('@') {
        Some((id, ver)) if id.is_empty() || ver.is_empty() => None,
        Some((id, ver)) => Some((id, Some(ver))),
        None => Some((reference, None)),
    }
}

fn collect_yaml(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), RegistryError> {
    let entries = std::fs::read_dir(dir).map_err(|source| RegistryError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| RegistryError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_yaml(&path, out)?;
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yaml") | Some("yml")
        ) {
            out.push(path);
        }
    }
    Ok(())
}
