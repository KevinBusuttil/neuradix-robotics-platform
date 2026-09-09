//! Offline identity drift example; does not execute or authorize a deployment.
use neuradix_graph::{ContractRegistry, from_file, validate, validate_with_registry};
fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut raw = from_file(&root.join("examples/reference-auv/deployment.yaml")).unwrap();
    let registry = ContractRegistry::load_dir(&root.join("contracts/standard")).unwrap();
    assert!(validate(&raw).resolved_identity().is_none());
    let original = validate_with_registry(&raw, &registry);
    assert!(original.is_valid(), "{:?}", original.issues());
    raw.spec.as_mut().unwrap().components[0].configuration = serde_yaml::from_str("{sample_period_us: 1000}").unwrap();
    let changed = validate_with_registry(&raw, &registry);
    assert!(changed.is_valid());
    assert_ne!(original.resolved_identity(), changed.resolved_identity());
    println!("original={}\nchanged={}", original.resolved_identity().unwrap(), changed.resolved_identity().unwrap());
}
