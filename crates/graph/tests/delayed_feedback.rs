//! Each scenario has an independent ten-second process deadline.
use neuradix_graph::{
    ConnectionDelay, ContractRegistry, RawDeployment, from_yaml_str, validate,
    validate_with_registry,
};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn raw() -> RawDeployment {
    from_yaml_str(
        include_str!("../../../examples/delayed-feedback/deployment.yaml"),
        Path::new("feedback"),
    )
    .unwrap()
}
fn value(s: &str) -> serde_yaml::Value {
    serde_yaml::from_str(s).unwrap()
}
fn delayed(ticks: u64) -> serde_yaml::Value {
    value(&format!(
        "{{ticks: {ticks}, unit: evaluation-ticks, initialization: require-seed}}"
    ))
}
fn has(raw: &RawDeployment, code: &str) -> bool {
    validate(raw).issues().iter().any(|i| i.code == code)
}
fn run(name: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "feedback_child", "--nocapture"])
        .env("NEURADIX_FEEDBACK_CASE", name)
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
scenario!(topology_and_subcycles);
scenario!(delay_boundaries_and_invalid);
scenario!(duplicates_and_conflicts);
scenario!(identity_versions_and_order);
scenario!(aggregate_and_count_boundaries);
scenario!(bounded_cycle_diagnostics);
#[test]
fn feedback_child() {
    let Ok(name) = std::env::var("NEURADIX_FEEDBACK_CASE") else {
        return;
    };
    match name.as_str() {
        "topology_and_subcycles" => {
            let mut a = raw();
            assert!(validate(&a).is_valid());
            a.spec.as_mut().unwrap().connections[1].delay = value("instantaneous");
            assert!(has(&a, "prohibited-cycle"));
            assert!(validate(&a).identity().is_none());
            let mut a = raw();
            let s = a.spec.as_mut().unwrap();
            // A disconnected instantaneous self-loop must not be hidden by the legal delayed loop.
            let mut c = s.components[0].clone();
            c.name = Some("other".into());
            c.requires = c.provides.clone();
            s.components.push(c);
            let mut e = s.connections[0].clone();
            e.from = Some("other".into());
            e.to = Some("other".into());
            s.connections.push(e);
            assert!(has(&a, "prohibited-cycle"));
            a.spec.as_mut().unwrap().connections[2].delay = delayed(1);
            assert!(validate(&a).is_valid());
            // The same component can participate in a legal cycle and an instantaneous subcycle.
            let s = a.spec.as_mut().unwrap();
            let provides = s.components[0].provides.clone();
            s.components[0].requires.extend(provides);
            let mut e = s.connections[0].clone();
            e.to = e.from.clone();
            s.connections.push(e);
            assert!(has(&a, "prohibited-cycle"));
            let mut a = raw();
            a.spec.as_mut().unwrap().components[1].runtime = Some("python".into());
            assert!(has(&a, "python-feeds-deterministic-path"));
            let mut a = raw();
            a.spec.as_mut().unwrap().components[0].role = Some("actuator".into());
            assert!(has(&a, "actuator-authority-bypass"));
            let mut a = raw();
            a.spec.as_mut().unwrap().connections[1].from = Some("missing".into());
            assert!(has(&a, "unknown-endpoint"));
        }
        "delay_boundaries_and_invalid" => {
            for n in [1, 1024] {
                let mut a = raw();
                a.spec.as_mut().unwrap().connections[1].delay = delayed(n);
                assert!(validate(&a).is_valid());
            }
            for n in [0, 1025, u64::MAX] {
                assert!(ConnectionDelay::new(n, "evaluation-ticks", "require-seed").is_err());
            }
            for text in [
                "null",
                "0",
                "[]",
                "!delay {ticks: 1, unit: evaluation-ticks, initialization: require-seed}",
                "{ticks: 0, unit: evaluation-ticks, initialization: require-seed}",
                "{ticks: -1, unit: evaluation-ticks, initialization: require-seed}",
                "{ticks: 1.0, unit: evaluation-ticks, initialization: require-seed}",
                "{ticks: 18446744073709551616, unit: evaluation-ticks, initialization: require-seed}",
                "{ticks: 1, unit: milliseconds, initialization: require-seed}",
                "{ticks: 1, unit: evaluation-ticks}",
                "{ticks: 1, unit: evaluation-ticks, initialization: zero}",
                "{ticks: 1, unit: evaluation-ticks, initialization: require-seed, extra: 1}",
                "{ticks: !integer 1, unit: evaluation-ticks, initialization: require-seed}",
            ] {
                if let Ok(v) = serde_yaml::from_str(text) {
                    let mut a = raw();
                    a.spec.as_mut().unwrap().connections[1].delay = v;
                    assert!(has(&a, "invalid-connection-delay"), "{text}");
                    assert!(validate(&a).identity().is_none());
                }
            }
            assert!(serde_yaml::from_str::<serde_yaml::Value>("{ticks: 1, ticks: 2}").is_err());
        }
        "duplicates_and_conflicts" => {
            let mut a = raw();
            let e = a.spec.as_ref().unwrap().connections[1].clone();
            a.spec.as_mut().unwrap().connections.push(e);
            assert!(has(&a, "duplicate-connection"));
            a.spec.as_mut().unwrap().connections[2].delay = delayed(2);
            assert!(has(&a, "conflicting-connection-delay"));
            a.spec.as_mut().unwrap().connections.reverse();
            assert!(has(&a, "conflicting-connection-delay"));
            assert!(validate(&a).identity().is_none());
        }
        "identity_versions_and_order" => {
            let reg = ContractRegistry::load_dir(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/standard"),
            )
            .unwrap();
            let mut a = raw();
            let r = validate_with_registry(&a, &reg);
            assert!(r.is_valid(), "{:?}", r.issues());
            assert_eq!(
                r.resolved_identity().unwrap(),
                "neuradix.deployment.resolved.v3:sha256:277a1301c175c3a6c1a465764b0866eb67b875c22df0aa0adb5a8e5ba0acd2db"
            );
            assert!(
                r.identity()
                    .unwrap()
                    .starts_with("neuradix.deployment.declared.v3:")
            );
            assert!(
                r.resolved_identity()
                    .unwrap()
                    .starts_with("neuradix.deployment.resolved.v3:")
            );
            a.spec.as_mut().unwrap().connections[1].delay =
                value("{initialization: require-seed, unit: evaluation-ticks, ticks: 0x1}");
            a.spec.as_mut().unwrap().connections.reverse();
            a.spec.as_mut().unwrap().components.reverse();
            assert_eq!(
                r.resolved_identity(),
                validate_with_registry(&a, &reg).resolved_identity()
            );
            a.spec.as_mut().unwrap().connections[0].delay = delayed(2);
            assert_ne!(r.identity(), validate(&a).identity());
            assert_ne!(
                r.resolved_identity(),
                validate_with_registry(&a, &reg).resolved_identity()
            );
            let mut a = raw();
            a.spec.as_mut().unwrap().connections.pop();
            a.spec.as_mut().unwrap().components[0].requires.clear();
            let old = validate_with_registry(&a, &reg);
            assert!(
                old.identity()
                    .unwrap()
                    .starts_with("neuradix.deployment.declared.v2:")
            );
            a.spec.as_mut().unwrap().connections[0].delay = value("instantaneous");
            assert_eq!(
                old.resolved_identity(),
                validate_with_registry(&a, &reg).resolved_identity()
            );
        }
        "aggregate_and_count_boundaries" => {
            let mut a = raw();
            let s = a.spec.as_mut().unwrap();
            let mut c = s.components[0].clone();
            c.requires = c.provides.clone();
            let e = s.connections[0].clone();
            s.components.clear();
            s.connections.clear();
            for n in 0..64 {
                let mut c = c.clone();
                c.name = Some(format!("c{n}"));
                s.components.push(c);
                let mut e = e.clone();
                e.from = Some(format!("c{n}"));
                e.to = e.from.clone();
                e.delay = delayed(1024);
                s.connections.push(e);
            }
            assert!(validate(&a).is_valid());
            let s = a.spec.as_mut().unwrap();
            let mut c = c.clone();
            c.name = Some("extra".into());
            s.components.push(c);
            let mut extra = e;
            extra.from = Some("extra".into());
            extra.to = extra.from.clone();
            extra.delay = delayed(1);
            s.connections.push(extra);
            assert!(has(&a, "delay-history-limit"));
            let mut a = raw();
            let s = a.spec.as_mut().unwrap();
            let mut c = s.components[0].clone();
            c.requires = c.provides.clone();
            let e = s.connections[0].clone();
            s.components.clear();
            s.connections.clear();
            for n in 0..64 {
                let mut c = c.clone();
                c.name = Some(format!("c{n}"));
                s.components.push(c);
            }
            for i in 0..64 {
                for j in 0..64 {
                    let mut e = e.clone();
                    e.from = Some(format!("c{i}"));
                    e.to = Some(format!("c{j}"));
                    e.delay = delayed(1);
                    s.connections.push(e);
                }
            }
            assert!(validate(&a).is_valid());
            let e = a.spec.as_ref().unwrap().connections[0].clone();
            a.spec.as_mut().unwrap().connections.push(e);
            assert!(!validate(&a).is_valid());
        }
        "bounded_cycle_diagnostics" => {
            let mut a = raw();
            let s = a.spec.as_mut().unwrap();
            let mut c = s.components[0].clone();
            c.requires = c.provides.clone();
            let e = s.connections[0].clone();
            s.components.clear();
            s.connections.clear();
            for n in 0..1024 {
                let mut c = c.clone();
                c.name = Some(format!("c{n:04}{}", "x".repeat(100)));
                s.components.push(c);
            }
            for n in 0..1024 {
                let mut e = e.clone();
                e.from = s.components[n].name.clone();
                e.to = s.components[(n + 1) % 1024].name.clone();
                s.connections.push(e);
            }
            let r = validate(&a);
            let issue = r
                .issues()
                .iter()
                .find(|i| i.code == "prohibited-cycle")
                .unwrap();
            assert!(issue.message.len() < 1024);
            a.spec.as_mut().unwrap().connections.reverse();
            a.spec.as_mut().unwrap().components.reverse();
            let r2 = validate(&a);
            assert_eq!(
                issue,
                r2.issues()
                    .iter()
                    .find(|i| i.code == "prohibited-cycle")
                    .unwrap()
            );
        }
        _ => panic!("unknown case"),
    }
}
