//! Externally timed identity and configuration boundary scenarios.
use neuradix_graph::{
    ComponentConfiguration as Config, ContractRegistry, from_yaml_str, validate,
    validate_with_registry,
};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn raw() -> neuradix_graph::RawDeployment {
    from_yaml_str(
        include_str!("../../../examples/reference-auv/deployment.yaml"),
        Path::new("test"),
    )
    .unwrap()
}
fn registry() -> ContractRegistry {
    ContractRegistry::load_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/standard"),
    )
    .unwrap()
}
fn config(s: &str) -> Result<Config, &'static str> {
    Config::from_value(&serde_yaml::from_str(s).map_err(|_| "parse")?)
}
fn run(name: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "identity_child", "--nocapture"])
        .env("NEURADIX_IDENTITY_CASE", name)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            assert!(s.success(), "{name}");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("deadline: {name}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
macro_rules! scenario {
    ($name:ident) => {
        #[test]
        fn $name() {
            run(stringify!($name));
        }
    };
}
scenario!(configuration_and_order);
scenario!(schema_layout_and_unsupported);
scenario!(configuration_boundaries);
scenario!(invalid_and_unresolved);
scenario!(source_and_duplicate_boundaries);
scenario!(aggregate_configuration_and_version_pin);

#[test]
fn identity_child() {
    let Ok(name) = std::env::var("NEURADIX_IDENTITY_CASE") else {
        return;
    };
    match name.as_str() {
        "configuration_and_order" => {
            let reg = registry();
            let mut a = raw();
            a.spec.as_mut().unwrap().components[0].configuration = serde_yaml::from_str(
                "{z: [1, true, null], a: {x: -9223372036854775808, y: 18446744073709551615}}",
            )
            .unwrap();
            let r = validate_with_registry(&a, &reg);
            assert!(r.is_valid());
            assert!(
                r.identity()
                    .unwrap()
                    .starts_with("neuradix.deployment.declared.v2:sha256:")
            );
            assert!(
                r.resolved_identity()
                    .unwrap()
                    .starts_with("neuradix.deployment.resolved.v2:sha256:")
            );
            let mut b = a.clone();
            let s = b.spec.as_mut().unwrap();
            s.nodes.reverse();
            s.components.reverse();
            s.connections.reverse();
            for c in &mut s.components {
                c.provides.reverse();
                c.requires.reverse();
            }
            assert_eq!(
                r.resolved_identity(),
                validate_with_registry(&b, &reg).resolved_identity()
            );
            a.spec.as_mut().unwrap().components[0].configuration = serde_yaml::from_str(
                "{a: {y: 18446744073709551615, x: -9223372036854775808}, z: [1, true, null]}",
            )
            .unwrap();
            assert_eq!(
                r.resolved_identity(),
                validate_with_registry(&a, &reg).resolved_identity()
            );
            a.spec.as_mut().unwrap().components[0].configuration =
                serde_yaml::from_str("{gain: 2}").unwrap();
            let changed = validate_with_registry(&a, &reg);
            assert_ne!(r.identity(), changed.identity());
            assert_ne!(r.resolved_identity(), changed.resolved_identity());
            assert_eq!(config("{x: 0x10}").unwrap(), config("{x: 16}").unwrap());
            assert_ne!(config("{x: [1,2]}").unwrap(), config("{x: [2,1]}").unwrap());
        }
        "schema_layout_and_unsupported" => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/standard");
            let dir =
                std::env::temp_dir().join(format!("neuradix-identity-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let files: Vec<_> = std::fs::read_dir(root)
                .unwrap()
                .flat_map(|dir| std::fs::read_dir(dir.unwrap().path()).unwrap())
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
                .collect();
            for file in &files {
                std::fs::copy(file, dir.join(file.file_name().unwrap())).unwrap();
            }
            let a = raw();
            let before = validate_with_registry(&a, &ContractRegistry::load_dir(&dir).unwrap());
            assert!(before.is_valid());
            // Change a supported field width in a referenced contract, same name/version.
            let file = files
                .iter()
                .find(|f| {
                    std::fs::read_to_string(f)
                        .unwrap()
                        .contains("name: sonar-range")
                })
                .unwrap();
            let text = std::fs::read_to_string(file).unwrap();
            assert!(text.contains("float64"));
            let dest = dir.join(file.file_name().unwrap());
            std::fs::write(&dest, text.replace("float64", "float32")).unwrap();
            let after = validate_with_registry(&a, &ContractRegistry::load_dir(&dir).unwrap());
            assert!(after.is_valid());
            assert_eq!(before.identity(), after.identity());
            assert_ne!(before.resolved_identity(), after.resolved_identity());
            let old = before
                .resolved()
                .iter()
                .find(|r| r.identifier.ends_with("/sonar-range"))
                .unwrap();
            let new = after
                .resolved()
                .iter()
                .find(|r| r.identifier.ends_with("/sonar-range"))
                .unwrap();
            assert_ne!(old.schema_id, new.schema_id);
            assert_ne!(old.wire_id, new.wire_id);
            assert_eq!(new.codec_id, "neuradix.scalar-le.v2");
            std::fs::write(&dest, text.replace("float64", "string")).unwrap();
            let rejected = validate_with_registry(&a, &ContractRegistry::load_dir(&dir).unwrap());
            assert!(!rejected.is_valid());
            assert!(rejected.identity().is_none());
            assert!(rejected.resolved_identity().is_none());
            assert!(
                rejected
                    .issues()
                    .iter()
                    .any(|i| i.code == "unsupported-contract-layout")
            );
            std::fs::copy(&dest, dir.join("duplicate.yaml")).unwrap();
            assert!(ContractRegistry::load_dir(&dir).is_err());
            std::fs::remove_dir_all(dir).unwrap();
        }
        "configuration_boundaries" => {
            let at = format!("{{\"x\":\"{}\"}}", "a".repeat(Config::MAX_BYTES - 8));
            assert_eq!(
                config(&at).unwrap().canonical_json().len(),
                Config::MAX_BYTES
            );
            assert!(config(&at.replace("aaa", "aaaa")).is_err());
            let values = |n| format!("{{x: [{}]}}", vec!["0"; n].join(","));
            assert!(config(&values(1022)).is_ok());
            assert!(config(&values(1023)).is_err());
            let nested = |n| format!("{{x: {}0{}}}", "[".repeat(n), "]".repeat(n));
            assert!(config(&nested(15)).is_ok());
            assert!(config(&nested(16)).is_err());
            for bad in [
                "{x: .nan}",
                "{x: .inf}",
                "{x: 1.0}",
                "{x: 1e300}",
                "{1: x}",
                "{x: !custom y}",
                "null",
                "[]",
                "{x: 18446744073709551616}",
            ] {
                assert!(config(bad).is_err(), "{bad}");
            }
            assert_eq!(
                config("{x: \"\\u0001\\n\\\"\\\\é\"}")
                    .unwrap()
                    .canonical_json(),
                "{\"x\":\"\\u0001\\n\\\"\\\\é\"}"
            );
        }
        "invalid_and_unresolved" => {
            let mut a = raw();
            assert!(validate(&a).resolved_identity().is_none());
            let reg = ContractRegistry::new();
            let r = validate_with_registry(&a, &reg);
            assert!(!r.is_valid());
            assert!(r.identity().is_none());
            assert!(r.resolved_identity().is_none());
            a.spec.as_mut().unwrap().components[0]
                .provides
                .push("missing/unwired@1.0.0".to_owned());
            assert!(
                validate_with_registry(&a, &registry())
                    .resolved_identity()
                    .is_none()
            );
            let mut a = raw();
            a.spec.as_mut().unwrap().components[0].configuration = serde_yaml::Value::Null;
            assert!(!validate_with_registry(&a, &registry()).is_valid());
            let mut a = raw();
            let s = a.spec.as_mut().unwrap();
            s.connections.push(s.connections[0].clone());
            let r = validate_with_registry(&a, &registry());
            assert!(r.resolved_identity().is_none());
            assert!(r.issues().iter().any(|i| i.code == "duplicate-connection"));
            let mut a = raw();
            let s = a.spec.as_mut().unwrap();
            let duplicate = s.components[0].provides[0].clone();
            s.components[0].provides.push(duplicate);
            assert!(
                validate_with_registry(&a, &registry())
                    .resolved_identity()
                    .is_none()
            );
        }
        "source_and_duplicate_boundaries" => {
            let text = include_str!("../../../examples/reference-auv/deployment.yaml");
            let exact = format!(
                "{text}#{}",
                "a".repeat(neuradix_graph::model::MAX_MANIFEST_BYTES - text.len() - 1)
            );
            assert!(from_yaml_str(&exact, Path::new("exact")).is_ok());
            let file =
                std::env::temp_dir().join(format!("neuradix-manifest-{}.yaml", std::process::id()));
            std::fs::write(&file, &exact).unwrap();
            assert!(neuradix_graph::from_file(&file).is_ok());
            // The detection byte can split a UTF-8 scalar; size still wins over decoding.
            std::fs::write(&file, exact.clone() + "é").unwrap();
            assert!(matches!(
                neuradix_graph::from_file(&file),
                Err(neuradix_graph::GraphError::Limit)
            ));
            std::fs::remove_file(file).unwrap();
            assert!(from_yaml_str(&(exact + "a"), Path::new("over")).is_err());
            let dup = text.replace(
                "executionClass: interactive",
                "configuration: {x: 1, x: 2}\n      executionClass: interactive",
            );
            assert!(from_yaml_str(&dup, Path::new("duplicate")).is_err());
            let mut a = raw();
            let s = a.spec.as_mut().unwrap();
            s.nodes = vec![s.nodes[0].clone(); 257];
            let r = validate(&a);
            assert!(r.identity().is_none());
            assert_eq!(r.issues()[0].code, "graph-count-limit");
        }
        "aggregate_configuration_and_version_pin" => {
            let empty = from_yaml_str("apiVersion: deploy.neuradix.io/v1alpha1\nkind: RobotDeployment\nmetadata: {name: empty}\nspec: {}", Path::new("empty")).unwrap();
            // Independent Python 3.11 json.dumps(sort_keys=True,separators=(',',':'))
            // + hashlib.sha256 over the documented empty resolved-v2 descriptor.
            assert_eq!(
                validate_with_registry(&empty, &ContractRegistry::new()).resolved_identity(),
                Some(
                    "neuradix.deployment.resolved.v2:sha256:8dfecbb59620f7ce755aab5afee5247feda95a6c238e778855af9b621067602e"
                )
            );
            let mut a = raw();
            let spec = a.spec.as_mut().unwrap();
            let mut component = spec.components[0].clone();
            component.configuration =
                serde_yaml::from_str(&format!("{{x: '{}' }}", "a".repeat(Config::MAX_BYTES - 8)))
                    .unwrap();
            spec.connections.clear();
            spec.components = (0..16)
                .map(|i| {
                    let mut c = component.clone();
                    c.name = format!("sensor{i}");
                    c
                })
                .collect();
            assert!(validate_with_registry(&a, &registry()).is_valid());
            component.name = "extra".to_owned();
            component.configuration = serde_yaml::from_str("{}").unwrap();
            a.spec.as_mut().unwrap().components.push(component);
            let report = validate_with_registry(&a, &registry());
            assert!(report.resolved_identity().is_none());
            assert!(
                report
                    .issues()
                    .iter()
                    .any(|i| i.code == "configuration-total-limit")
            );
            let unknown = "apiVersion: deploy.neuradix.io/v1alpha1\nkind: RobotDeployment\nmetadata: {name: empty}\nspec: {unknown: ignored}";
            assert!(from_yaml_str(unknown, Path::new("unknown")).is_err());
        }
        _ => panic!("unknown case"),
    }
}
