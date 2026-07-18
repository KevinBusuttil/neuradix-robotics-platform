//! Content-addressed deployment identity (§28.4 production immutability).

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::model::Deployment;

/// Compute the `sha256:<hex>` identity of a deployment's normative content.
///
/// The identity covers `name`, `profile`, and the nodes, components and
/// connections (with provides/requires and connections sorted), so it is stable
/// regardless of authoring order. A production deployment can be pinned to this.
pub fn deployment_identity(deployment: &Deployment) -> String {
    let value = canonical_value(deployment);
    let bytes = serde_json::to_vec(&sort_keys(value)).expect("canonical JSON cannot fail");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("sha256:{}", to_hex(&hasher.finalize()))
}

fn canonical_value(deployment: &Deployment) -> Value {
    let mut nodes: Vec<Value> = deployment
        .nodes
        .iter()
        .map(|n| json!({ "name": n.name, "target": n.target }))
        .collect();
    nodes.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    let mut components: Vec<Value> = deployment
        .components
        .iter()
        .map(|c| {
            let mut provides: Vec<String> = c.provides.clone();
            provides.sort();
            let mut requires: Vec<String> = c.requires.clone();
            requires.sort();
            json!({
                "name": c.name,
                "node": c.node,
                "executionClass": c.execution_class.as_str(),
                "runtime": c.runtime.as_str(),
                "role": c.role.as_str(),
                "provides": provides,
                "requires": requires,
            })
        })
        .collect();
    components.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    let mut connections: Vec<Value> = deployment
        .connections
        .iter()
        .map(|c| json!({ "from": c.from, "to": c.to, "contract": c.contract }))
        .collect();
    connections.sort_by(|a, b| {
        (a["from"].as_str(), a["to"].as_str(), a["contract"].as_str()).cmp(&(
            b["from"].as_str(),
            b["to"].as_str(),
            b["contract"].as_str(),
        ))
    });

    json!({
        "name": deployment.name,
        "profile": deployment.profile,
        "nodes": nodes,
        "components": components,
        "connections": connections,
    })
}

fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            let mut sorted = Map::new();
            for (k, v) in entries {
                sorted.insert(k, sort_keys(v));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
