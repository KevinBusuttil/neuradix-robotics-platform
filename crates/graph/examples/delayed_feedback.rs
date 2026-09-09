//! Demonstrate offline feedback admission and instantaneous-cycle rejection.
use neuradix_graph::{ContractRegistry, from_file, validate_with_registry};
fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut raw = from_file(&root.join("examples/delayed-feedback/deployment.yaml")).unwrap();
    let registry = ContractRegistry::load_dir(&root.join("contracts/standard")).unwrap();
    let report = validate_with_registry(&raw, &registry);
    assert!(report.is_valid(), "{:?}", report.issues());
    println!("offline delayed loop: {}", report.resolved_identity().unwrap());
    raw.spec.as_mut().unwrap().connections[1].delay = serde_yaml::Value::String("instantaneous".to_owned());
    let rejected = validate_with_registry(&raw, &registry);
    assert!(!rejected.is_valid());
    assert!(rejected.resolved_identity().is_none());
    println!("instantaneous loop rejected: {:?}", rejected.issues());
}
