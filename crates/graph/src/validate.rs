//! Deployment graph validation: build a typed model and check topology + policy
//! before runtime ("contracts before connectivity", §3.1, §28.2).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::error::GraphError;
use crate::identity::deployment_identity;
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
    pub identity: String,
    /// All validation issues, in the order discovered.
    pub issues: Vec<GraphIssue>,
    /// Contract references resolved against a registry, sorted by reference.
    /// Empty when validation ran without a registry.
    pub resolved: Vec<ResolvedContract>,
}

impl GraphReport {
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
    let deployment = build(raw, &mut issues);
    check(&deployment, &mut issues);
    let resolved = resolve_contracts(&deployment, registry, &mut issues);
    let identity = deployment_identity(&deployment);
    GraphReport {
        identity,
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
    let references: BTreeSet<&str> = d.connections.iter().map(|c| c.contract.as_str()).collect();

    let mut resolved = Vec::new();
    for reference in references {
        let path = format!("contract `{reference}`");
        match registry.resolve(reference) {
            Resolution::Resolved(entry) => resolved.push(ResolvedContract {
                reference: reference.to_owned(),
                identifier: entry.identifier.clone(),
                version: entry.version.clone(),
                schema_id: entry.schema_id.clone(),
            }),
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
        components.push(Component {
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
        connections.push(Connection { from, to, contract });
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

    // Prohibited cycles in the topology.
    if let Some(cycle) = find_cycle(d, &comp_by_name) {
        error(
            issues,
            "prohibited-cycle",
            "spec.connections",
            format!(
                "prohibited cycle in the component graph: {}",
                cycle.join(" -> ")
            ),
        );
    }
}

/// Find a directed cycle in the connection graph, returning the cycle path.
fn find_cycle(d: &Deployment, comp_by_name: &HashMap<&str, &Component>) -> Option<Vec<String>> {
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for conn in &d.connections {
        if comp_by_name.contains_key(conn.from.as_str())
            && comp_by_name.contains_key(conn.to.as_str())
        {
            adjacency
                .entry(conn.from.as_str())
                .or_default()
                .push(conn.to.as_str());
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Grey,
        Black,
    }
    let mut color: HashMap<&str, Color> = comp_by_name.keys().map(|&k| (k, Color::White)).collect();
    let mut stack: Vec<&str> = Vec::new();

    fn dfs<'a>(
        node: &'a str,
        adjacency: &HashMap<&'a str, Vec<&'a str>>,
        color: &mut HashMap<&'a str, Color>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        color.insert(node, Color::Grey);
        stack.push(node);
        if let Some(neighbours) = adjacency.get(node) {
            for &next in neighbours {
                match color.get(next).copied().unwrap_or(Color::White) {
                    Color::White => {
                        if let Some(cycle) = dfs(next, adjacency, color, stack) {
                            return Some(cycle);
                        }
                    }
                    Color::Grey => {
                        // Back edge: reconstruct the cycle from the stack.
                        let start = stack.iter().position(|&n| n == next).unwrap_or(0);
                        let mut cycle: Vec<String> =
                            stack[start..].iter().map(|s| s.to_string()).collect();
                        cycle.push(next.to_string());
                        return Some(cycle);
                    }
                    Color::Black => {}
                }
            }
        }
        stack.pop();
        color.insert(node, Color::Black);
        None
    }

    let mut names: Vec<&str> = comp_by_name.keys().copied().collect();
    names.sort_unstable(); // deterministic traversal order
    for name in names {
        if color.get(name).copied().unwrap_or(Color::White) == Color::White
            && let Some(cycle) = dfs(name, &adjacency, &mut color, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
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
