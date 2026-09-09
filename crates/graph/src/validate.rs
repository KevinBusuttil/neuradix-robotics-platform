//! Deployment graph validation: build a typed model and check topology + policy
//! before runtime ("contracts before connectivity", §3.1, §28.2).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::{ComponentConfiguration, ConnectionDelay};
use crate::error::GraphError;
use crate::identity::{declared_identity, resolved_identity};
use crate::model::{
    Component, Connection, Deployment, ExecutionClass, Node, RawDeployment, Role, Runtime,
    SUPPORTED_API_VERSION, from_file, from_yaml_str,
};
use crate::registry::{ContractRegistry, Resolution};

/// The severity of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A problem that makes the deployment invalid.
    Error,
    /// A non-fatal advisory.
    Warning,
}

impl Severity {
    /// The canonical lowercase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// A single validation issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphIssue {
    /// Severity.
    pub severity: Severity,
    /// Stable machine-readable code (kebab-case).
    pub code: String,
    /// Dotted/pathy location the issue relates to.
    pub path: String,
    /// Human-readable description.
    pub message: String,
}

/// A contract reference that resolved to a real registered schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContract {
    /// Versioned codec computed from the validated schema.
    pub codec_id: String,
    /// Full scalar wire identity.
    pub wire_id: String,
    /// The reference as written in the deployment (`namespace/name[@version]`).
    pub reference: String,
    /// The resolved contract's `namespace/name` identifier.
    pub identifier: String,
    /// The resolved version.
    pub version: String,
    /// The resolved `sha256:` schema identity.
    pub schema_id: String,
}

/// The result of validating a deployment.
#[derive(Debug, Clone)]
pub struct GraphReport {
    /// Content-addressed deployment identity.
    identity: Option<String>,
    resolved_identity: Option<String>,
    /// All validation issues, in the order discovered.
    issues: Vec<GraphIssue>,
    /// Contract references resolved against a registry, sorted by reference.
    /// Empty when validation ran without a registry.
    resolved: Vec<ResolvedContract>,
}

impl GraphReport {
    /// Versioned declared-content identity; absent for invalid graphs.
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }
    /// Resolved identity only after complete registry and topology validation.
    pub fn resolved_identity(&self) -> Option<&str> {
        self.resolved_identity.as_deref()
    }
    /// Immutable validation evidence; callers cannot clear issues to forge validity.
    pub fn issues(&self) -> &[GraphIssue] {
        &self.issues
    }
    /// Immutable computed bindings; no caller-supplied digest insertion.
    pub fn resolved(&self) -> &[ResolvedContract] {
        &self.resolved
    }

    /// Whether the deployment is valid (no error-severity issues).
    pub fn is_valid(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    /// The number of error-severity issues.
    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    /// The number of warning-severity issues.
    pub fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .count()
    }
}

/// Parse and validate a deployment from a YAML string.
pub fn from_yaml(source: &str, path: &Path) -> Result<GraphReport, GraphError> {
    let raw = from_yaml_str(source, path)?;
    Ok(validate(&raw))
}

/// Read, parse and validate a deployment file.
pub fn load_file(path: &Path) -> Result<GraphReport, GraphError> {
    let raw = from_file(path)?;
    Ok(validate(&raw))
}

/// Validate a parsed raw deployment, checking topology and policy only.
///
/// Contract references are checked for consistency (a producer provides and a
/// consumer requires the same reference) but not resolved against real
/// contracts. Use [`validate_with_registry`] to additionally prove every
/// reference resolves to a registered schema.
pub fn validate(raw: &RawDeployment) -> GraphReport {
    validate_inner(raw, None)
}

/// Validate a parsed raw deployment and resolve every wired contract reference
/// against `registry`, pinning the schema identity each resolves to.
///
/// In addition to the [`validate`] checks, an unresolved reference is an
/// `unknown-contract` error, a reference to a missing version is
/// `unknown-contract-version`, and an unpinned reference to a multi-version
/// contract is `ambiguous-contract` (§28.4 immutability wants pinned schemas).
pub fn validate_with_registry(raw: &RawDeployment, registry: &ContractRegistry) -> GraphReport {
    validate_inner(raw, Some(registry))
}

fn validate_inner(raw: &RawDeployment, registry: Option<&ContractRegistry>) -> GraphReport {
    let mut issues = Vec::new();
    if raw.spec.as_ref().is_some_and(|s| {
        s.nodes.len() > 256
            || s.components.len() > 1024
            || s.connections.len() > 4096
            || s.components
                .iter()
                .any(|c| c.provides.len() > 256 || c.requires.len() > 256)
    }) {
        error(
            &mut issues,
            "graph-count-limit",
            "spec",
            "exceeds bounded identity graph counts".to_owned(),
        );
        return GraphReport {
            identity: None,
            resolved_identity: None,
            issues,
            resolved: Vec::new(),
        };
    }
    let deployment = build(raw, &mut issues);
    let total_configuration: usize = deployment
        .components
        .iter()
        .map(|c| c.configuration.canonical_json().len())
        .sum();
    if total_configuration > 1 << 20 {
        error(
            &mut issues,
            "configuration-total-limit",
            "spec.components",
            "total canonical configuration exceeds 1 MiB".to_owned(),
        );
    }
    check(&deployment, &mut issues);
    let resolved = resolve_contracts(&deployment, registry, &mut issues);
    let valid = !issues.iter().any(|i| i.severity == Severity::Error);
    let identity = valid.then(|| declared_identity(&deployment));
    let resolved_identity =
        (valid && registry.is_some()).then(|| resolved_identity(&deployment, &resolved));
    GraphReport {
        identity,
        resolved_identity,
        issues,
        resolved,
    }
}

/// Resolve every distinct contract reference wired by a connection against the
/// registry, collecting the resolved schemas and emitting an issue per failure.
fn resolve_contracts(
    d: &Deployment,
    registry: Option<&ContractRegistry>,
    issues: &mut Vec<GraphIssue>,
) -> Vec<ResolvedContract> {
    let Some(registry) = registry else {
        return Vec::new();
    };

    // Distinct references, in deterministic order.
    let references: BTreeSet<&str> = d
        .connections
        .iter()
        .map(|c| c.contract.as_str())
        .chain(
            d.components
                .iter()
                .flat_map(|c| c.provides.iter().chain(&c.requires).map(String::as_str)),
        )
        .collect();

    let mut resolved = Vec::new();
    for reference in references {
        let path = format!("contract `{reference}`");
        match registry.resolve(reference) {
            Resolution::Resolved(entry) => match &entry.layout {
                Ok(layout) => resolved.push(ResolvedContract {
                    reference: reference.to_owned(),
                    identifier: entry.identifier.clone(),
                    version: entry.version.clone(),
                    schema_id: entry.schema_id.clone(),
                    codec_id: layout.codec_id.clone(),
                    wire_id: layout.wire_id.clone(),
                }),
                Err(reason) => error(issues, "unsupported-contract-layout", &path, reason.clone()),
            },
            Resolution::UnknownContract => error(
                issues,
                "unknown-contract",
                &path,
                format!("reference `{reference}` does not resolve to a registered contract"),
            ),
            Resolution::UnknownVersion => error(
                issues,
                "unknown-contract-version",
                &path,
                format!("reference `{reference}` names a version that is not registered"),
            ),
            Resolution::Ambiguous(versions) => error(
                issues,
                "ambiguous-contract",
                &path,
                format!(
                    "reference `{reference}` matches several versions ({}); pin one with `@`",
                    versions.join(", ")
                ),
            ),
            Resolution::Malformed => error(
                issues,
                "malformed-contract-reference",
                &path,
                format!("reference `{reference}` is not a valid `namespace/name[@version]`"),
            ),
        }
    }
    resolved
}

// ---------------------------------------------------------------------------
// Build a best-effort typed model, collecting structural issues.
// ---------------------------------------------------------------------------

fn build(raw: &RawDeployment, issues: &mut Vec<GraphIssue>) -> Deployment {
    match raw.api_version.as_deref() {
        Some(SUPPORTED_API_VERSION) => {}
        Some(other) => error(
            issues,
            "unsupported-api-version",
            "apiVersion",
            format!("unsupported apiVersion `{other}`; expected `{SUPPORTED_API_VERSION}`"),
        ),
        None => error(
            issues,
            "missing-field",
            "apiVersion",
            "is required".to_owned(),
        ),
    }
    match raw.kind.as_deref() {
        Some("RobotDeployment") => {}
        Some(other) => error(
            issues,
            "unsupported-kind",
            "kind",
            format!("unsupported kind `{other}`; expected `RobotDeployment`"),
        ),
        None => error(issues, "missing-field", "kind", "is required".to_owned()),
    }

    let name = raw
        .metadata
        .as_ref()
        .and_then(|m| m.name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            error(
                issues,
                "missing-field",
                "metadata.name",
                "is required".to_owned(),
            );
            String::new()
        });

    let spec = raw.spec.as_ref();
    let profile = spec.and_then(|s| s.profile.clone());

    let mut nodes = Vec::new();
    let mut seen_nodes = HashSet::new();
    for (i, n) in spec
        .map(|s| s.nodes.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        let base = format!("spec.nodes[{i}]");
        let Some(nname) = require(&n.name, &format!("{base}.name"), issues) else {
            continue;
        };
        let target = n.target.clone().unwrap_or_else(|| {
            error(
                issues,
                "missing-field",
                &format!("{base}.target"),
                "is required".to_owned(),
            );
            String::new()
        });
        if !seen_nodes.insert(nname.clone()) {
            error(
                issues,
                "duplicate-node",
                &base,
                format!("duplicate node name `{nname}`"),
            );
        }
        nodes.push(Node {
            name: nname,
            target,
        });
    }

    let mut components = Vec::new();
    let mut seen_components = HashSet::new();
    for (i, c) in spec
        .map(|s| s.components.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        let base = format!("spec.components[{i}]");
        let Some(cname) = require(&c.name, &format!("{base}.name"), issues) else {
            continue;
        };
        let node = c.node.clone().unwrap_or_else(|| {
            error(
                issues,
                "missing-field",
                &format!("{base}.node"),
                "is required".to_owned(),
            );
            String::new()
        });
        let execution_class = parse_enum(
            c.execution_class.as_deref(),
            ExecutionClass::parse,
            ExecutionClass::ALL,
            &format!("{base}.executionClass"),
            "execution class",
            issues,
        )
        .unwrap_or(ExecutionClass::Interactive);
        let runtime = c
            .runtime
            .as_deref()
            .map(|s| {
                parse_enum(
                    Some(s),
                    Runtime::parse,
                    Runtime::ALL,
                    &format!("{base}.runtime"),
                    "runtime",
                    issues,
                )
                .unwrap_or(Runtime::Rust)
            })
            .unwrap_or(Runtime::Rust);
        let role = c
            .role
            .as_deref()
            .map(|s| {
                parse_enum(
                    Some(s),
                    Role::parse,
                    Role::ALL,
                    &format!("{base}.role"),
                    "role",
                    issues,
                )
                .unwrap_or(Role::Normal)
            })
            .unwrap_or(Role::Normal);
        if !seen_components.insert(cname.clone()) {
            error(
                issues,
                "duplicate-component",
                &base,
                format!("duplicate component name `{cname}`"),
            );
        }
        let configuration =
            ComponentConfiguration::from_value(&c.configuration).unwrap_or_else(|reason| {
                error(
                    issues,
                    "invalid-configuration",
                    &format!("{base}.configuration"),
                    reason.to_owned(),
                );
                ComponentConfiguration::default()
            });
        for references in [&c.provides, &c.requires] {
            let mut seen = HashSet::new();
            if references.iter().any(|r| !seen.insert(r)) {
                error(
                    issues,
                    "duplicate-contract-reference",
                    &base,
                    "duplicate provides/requires reference".to_owned(),
                );
            }
        }
        components.push(Component {
            configuration,
            name: cname,
            node,
            execution_class,
            runtime,
            role,
            provides: c.provides.clone(),
            requires: c.requires.clone(),
        });
    }

    let mut connections = Vec::new();
    let mut seen_connections = HashMap::new();
    for (i, c) in spec
        .map(|s| s.connections.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        let base = format!("spec.connections[{i}]");
        let (Some(from), Some(to), Some(contract)) = (
            require(&c.from, &format!("{base}.from"), issues),
            require(&c.to, &format!("{base}.to"), issues),
            require(&c.contract, &format!("{base}.contract"), issues),
        ) else {
            continue;
        };
        let delay = ConnectionDelay::from_value(&c.delay).unwrap_or_else(|reason| {
            error(issues, "invalid-connection-delay", &format!("{base}.delay"), reason.to_owned());
            ConnectionDelay::default()
        });
        if let Some(previous) = seen_connections.insert((from.clone(), to.clone(), contract.clone()), delay) {
            let code = if previous == delay { "duplicate-connection" } else { "conflicting-connection-delay" };
            error(issues, code, &base, "duplicate connection or conflicting delay declaration".to_owned());
        }
        connections.push(Connection { from, to, contract, delay });
    }

    Deployment {
        name,
        profile,
        nodes,
        components,
        connections,
    }
}

// ---------------------------------------------------------------------------
// Graph-level checks.
// ---------------------------------------------------------------------------

fn check(d: &Deployment, issues: &mut Vec<GraphIssue>) {
    let node_names: HashSet<&str> = d.nodes.iter().map(|n| n.name.as_str()).collect();
    let comp_by_name: HashMap<&str, &Component> =
        d.components.iter().map(|c| (c.name.as_str(), c)).collect();

    // Placement + Python-on-deterministic-path checks per component.
    for c in &d.components {
        if !c.node.is_empty() && !node_names.contains(c.node.as_str()) {
            error(
                issues,
                "unknown-node",
                &format!("component `{}`", c.name),
                format!("placed on unknown node `{}`", c.node),
            );
        }
        if c.runtime == Runtime::Python && c.execution_class.is_deterministic_path() {
            error(
                issues,
                "python-in-deterministic-path",
                &format!("component `{}`", c.name),
                format!(
                    "Python components must not run in the `{}` (deterministic) execution class",
                    c.execution_class.as_str()
                ),
            );
        }
    }

    // Connection-level checks.
    for conn in &d.connections {
        let from = comp_by_name.get(conn.from.as_str());
        let to = comp_by_name.get(conn.to.as_str());
        if from.is_none() {
            error(
                issues,
                "unknown-endpoint",
                &format!("connection {}->{}", conn.from, conn.to),
                format!("unknown producer component `{}`", conn.from),
            );
        }
        if to.is_none() {
            error(
                issues,
                "unknown-endpoint",
                &format!("connection {}->{}", conn.from, conn.to),
                format!("unknown consumer component `{}`", conn.to),
            );
        }
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };

        if !from.provides.contains(&conn.contract) {
            error(
                issues,
                "provider-mismatch",
                &format!("connection {}->{}", conn.from, conn.to),
                format!("`{}` does not provide `{}`", conn.from, conn.contract),
            );
        }
        if !to.requires.contains(&conn.contract) {
            error(
                issues,
                "consumer-mismatch",
                &format!("connection {}->{}", conn.from, conn.to),
                format!("`{}` does not require `{}`", conn.to, conn.contract),
            );
        }
        if from.runtime == Runtime::Python && to.execution_class.is_deterministic_path() {
            error(
                issues,
                "python-feeds-deterministic-path",
                &format!("connection {}->{}", conn.from, conn.to),
                format!(
                    "Python component `{}` feeds deterministic component `{}`",
                    conn.from, conn.to
                ),
            );
        }
        if to.role == Role::Actuator && from.role != Role::Safety {
            error(
                issues,
                "actuator-authority-bypass",
                &format!("connection {}->{}", conn.from, conn.to),
                format!(
                    "actuator `{}` is commanded by `{}` without passing through Safety (§16.1)",
                    conn.to, conn.from
                ),
            );
        }
    }

    // Every requirement must have a provider connection.
    for c in &d.components {
        for req in &c.requires {
            let provided = d
                .connections
                .iter()
                .any(|conn| conn.to == c.name && &conn.contract == req);
            if !provided {
                error(
                    issues,
                    "unresolved-requirement",
                    &format!("component `{}`", c.name),
                    format!("requires `{req}` but no connection provides it"),
                );
            }
        }
    }

    // Safety-policy presence: an actuator requires a Safety authority.
    let has_actuator = d.components.iter().any(|c| c.role == Role::Actuator);
    let has_safety = d.components.iter().any(|c| c.role == Role::Safety);
    if has_actuator && !has_safety {
        error(
            issues,
            "missing-safety",
            "spec.components",
            "deployment has actuators but no Safety authority component".to_owned(),
        );
    }

    let total_ticks: u64 = d.connections.iter().map(|c| u64::from(c.delay.ticks())).sum();
    // Connection count and per-edge bounds make the u64 sum representable.
    if total_ticks > ConnectionDelay::MAX_TOTAL_TICKS {
        error(issues, "delay-history-limit", "spec.connections", "sum of delay ticks exceeds 65536 history slots".to_owned());
    }
    // Only instantaneous edges participate in same-tick dependencies.
    if let Some(cycle) = find_cycle(d, &comp_by_name) {
        error(
            issues,
            "prohibited-cycle",
            "spec.connections",
            cycle_summary(&cycle),
        );
    }
}

/// Iterative deterministic DFS of the instantaneous subgraph. All edges still
/// undergo endpoint/contract/policy checks before this analysis.
fn find_cycle<'a>(d: &'a Deployment, comp_by_name: &HashMap<&str, &Component>) -> Option<Vec<&'a str>> {
    let mut names: Vec<&str> = d.components.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for conn in &d.connections {
        if conn.delay.is_instantaneous() && comp_by_name.contains_key(conn.from.as_str()) && comp_by_name.contains_key(conn.to.as_str()) {
            adjacency.entry(conn.from.as_str()).or_default().push(conn.to.as_str());
        }
    }
    for neighbours in adjacency.values_mut() { neighbours.sort_unstable(); neighbours.dedup(); }
    // 0=unvisited, 1=active, 2=finished. Stack bounded by admitted component count.
    let mut color: HashMap<&str, u8> = names.iter().map(|&n| (n, 0)).collect();
    for root in names {
        if color[root] != 0 { continue; }
        let mut stack = vec![(root, 0usize)];
        color.insert(root, 1);
        while let Some((node, next)) = stack.last_mut() {
            let neighbours = adjacency.get(node).map_or(&[][..], Vec::as_slice);
            if *next == neighbours.len() {
                color.insert(*node, 2);
                stack.pop();
                continue;
            }
            let target = neighbours[*next];
            *next += 1;
            match color[target] {
                0 => { color.insert(target, 1); stack.push((target, 0)); }
                1 => {
                    let start = stack.iter().position(|(n,_)| *n == target).expect("active node");
                    let mut cycle: Vec<_> = stack[start..].iter().map(|(n,_)| *n).collect();
                    cycle.push(target);
                    return Some(cycle);
                }
                _ => {}
            }
        }
    }
    None
}

/// At most 16 names of at most 32 UTF-8 bytes each, plus fixed framing. The full
/// witness is transient and bounded by the admitted 1024 components.
fn cycle_summary(cycle: &[&str]) -> String {
    let names: Vec<_> = cycle.iter().take(16).map(|name| {
        let mut end = name.len().min(32);
        while !name.is_char_boundary(end) { end -= 1; }
        format!("{}{}", &name[..end], if end < name.len() { "…" } else { "" })
    }).collect();
    format!("instantaneous cycle ({} vertices including closure): {}{}", cycle.len(), names.join(" -> "), if cycle.len() > 16 { " -> …" } else { "" })
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn error(issues: &mut Vec<GraphIssue>, code: &str, path: &str, message: String) {
    issues.push(GraphIssue {
        severity: Severity::Error,
        code: code.to_owned(),
        path: path.to_owned(),
        message,
    });
}

fn require(value: &Option<String>, path: &str, issues: &mut Vec<GraphIssue>) -> Option<String> {
    match value {
        Some(s) if !s.trim().is_empty() => Some(s.clone()),
        _ => {
            error(issues, "missing-field", path, "is required".to_owned());
            None
        }
    }
}

fn parse_enum<T>(
    value: Option<&str>,
    parse: fn(&str) -> Option<T>,
    all: &[&str],
    path: &str,
    what: &str,
    issues: &mut Vec<GraphIssue>,
) -> Option<T> {
    match value {
        Some(s) => match parse(s) {
            Some(v) => Some(v),
            None => {
                error(
                    issues,
                    "unknown-value",
                    path,
                    format!("unknown {what} `{s}`; supported: {}", all.join(", ")),
                );
                None
            }
        },
        None => {
            error(issues, "missing-field", path, "is required".to_owned());
            None
        }
    }
}
