//! Linux kernel enforcement, exercised in externally timed subprocesses.
#![cfg(all(target_os = "linux", not(target_env = "uclibc")))]

use neuradix_python::{
    HeartbeatPolicy, IoLimits, ObservedExit, PythonWorker, ResourceLimits, ResourceStage, Timeouts,
    WorkerConfig, WorkerError, WorkerFailure, WorkerSupervisor,
};
use neuradix_runtime::HealthState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const LAUNCHER: &str = env!("CARGO_BIN_EXE_neuradix-python-launcher");
const RESERVE: Duration = Duration::from_millis(50);
const TOLERANCE: Duration = Duration::from_millis(300);
fn config() -> WorkerConfig {
    WorkerConfig::new(
        "python3",
        format!("{}/tests/workers/resources.py", env!("CARGO_MANIFEST_DIR")),
    )
    .with_resource_launcher(LAUNCHER)
    .unwrap()
    .with_python_path(format!("{}/../../python", env!("CARGO_MANIFEST_DIR")))
    .with_timeouts(
        Timeouts::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_millis(300),
            RESERVE,
        )
        .unwrap(),
    )
    .with_heartbeat(HeartbeatPolicy::new(Duration::from_secs(10), Duration::from_secs(1)).unwrap())
}
fn cause(error: &WorkerError) -> &WorkerError {
    match error {
        WorkerError::Cleanup { cause: inner, .. } => cause(inner),
        other => other,
    }
}
fn bounded(start: Instant, budget: Duration) {
    assert!(
        start.elapsed() <= budget + TOLERANCE,
        "elapsed {:?}, budget {budget:?}",
        start.elapsed()
    );
}
fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("neuradix-resources-{}-{name}", std::process::id()))
}
fn not_running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.rsplit_once(") ")
            .is_some_and(|(_, tail)| tail.starts_with('Z'))
    })
}
fn await_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !not_running(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        not_running(pid),
        "worker did not exit within kernel-enforcement tolerance"
    );
}
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
fn assert_policy(value: &Value, limits: ResourceLimits) {
    assert_eq!(
        value["cpu"],
        json!([limits.cpu_seconds(), limits.cpu_seconds()])
    );
    assert_eq!(
        value["as"],
        json!([limits.address_space_bytes(), limits.address_space_bytes()])
    );
}
fn external(case: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "resource_child", "--nocapture"])
        .env("NEURADIX_RESOURCE_CASE", case)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "resource scenario {case} failed");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("external resource test timeout: {case}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
macro_rules! scenarios {
    ($($case:ident),+ $(,)?) => { $(#[test] fn $case() { external(stringify!($case)); })+ };
}
scenarios!(
    valid_and_page_rounded,
    one_message_bootstrap,
    cpu_hard_limit,
    cpu_is_not_wall_time,
    address_space_failure_and_recovery,
    inherited_hard_limits,
    cannot_raise_limits,
    missing_launcher,
    setup_failure_precedes_worker,
    bootstrap_deadline_and_storage,
    failed_setup_restart_budget,
    cpu_failure_restart_budget,
    generic_exit_is_not_exhaustion,
    privileged_launcher_rejected,
    worker_cannot_reconfigure,
    local_control_during_cpu_exhaustion
);

#[test]
fn resource_child() {
    let Ok(case) = std::env::var("NEURADIX_RESOURCE_CASE") else {
        return;
    };
    assert!(
        Command::new("python3")
            .arg("--version")
            .status()
            .unwrap()
            .success()
    );
    match case.as_str() {
        "valid_and_page_rounded" => {
            use nix::sys::resource::{Resource, getrlimit};
            let before = (
                getrlimit(Resource::RLIMIT_CPU).unwrap(),
                getrlimit(Resource::RLIMIT_AS).unwrap(),
            );
            let requested = ResourceLimits::new(3, 268_435_457).unwrap();
            let mut worker = PythonWorker::launch(&config().with_resources(requested)).unwrap();
            assert_eq!(
                worker.applied_resources().address_space_bytes(),
                268_435_456
            );
            for _ in 0..4 {
                let out = worker.send(&Value::Null).unwrap();
                assert_policy(&out, worker.applied_resources());
            }
            worker.shutdown();
            assert_eq!(
                before,
                (
                    getrlimit(Resource::RLIMIT_CPU).unwrap(),
                    getrlimit(Resource::RLIMIT_AS).unwrap()
                )
            );
        }
        "one_message_bootstrap" => {
            let cfg = config()
                .with_limits(IoLimits::new(64, 64, 64, 1).unwrap())
                .with_heartbeat(
                    HeartbeatPolicy::new(Duration::from_millis(30), Duration::from_millis(250))
                        .unwrap(),
                );
            let mut worker = PythonWorker::launch(&cfg).unwrap();
            std::thread::sleep(
                worker
                    .heartbeat_due()
                    .saturating_duration_since(Instant::now()),
            );
            assert!(worker.check_heartbeat().unwrap());
            let stats = worker.io_stats();
            assert!(
                stats.queued_bytes <= 64
                    && stats.queued_messages <= 1
                    && stats.outgoing_bytes <= 64
            );
        }
        "cpu_hard_limit" => {
            let mut worker = PythonWorker::launch(
                &config().with_resources(ResourceLimits::new(1, 268_435_456).unwrap()),
            )
            .unwrap();
            let start = Instant::now();
            assert_eq!(worker.send(&json!("cpu")).unwrap(), "started");
            await_exit(worker.process_id());
            assert_eq!(worker.health(), HealthState::Unavailable);
            assert_eq!(
                worker.observed_exit(),
                Some(ObservedExit::Signal {
                    signal: 9,
                    core_dumped: false
                })
            );
            assert_eq!(worker.last_failure(), Some(WorkerFailure::Exited));
            bounded(start, Duration::from_secs(5));
            assert!(worker.cleanup_report().is_some());
        }
        "cpu_is_not_wall_time" => {
            let mut worker = PythonWorker::launch(
                &config().with_resources(ResourceLimits::new(1, 268_435_456).unwrap()),
            )
            .unwrap();
            assert_policy(
                &worker.send(&json!("idle")).unwrap(),
                worker.applied_resources(),
            );
            assert_eq!(worker.health(), HealthState::Healthy);
            assert!(worker.observed_exit().is_none());
        }
        "address_space_failure_and_recovery" => {
            let mut worker = PythonWorker::launch(
                &config().with_resources(ResourceLimits::new(3, 67_108_864).unwrap()),
            )
            .unwrap();
            assert!(
                matches!(worker.send(&json!("memory")), Err(WorkerError::Remote(message)) if message == "MemoryError")
            );
            assert_eq!(worker.health(), HealthState::Healthy);
            assert_eq!(worker.send(&json!("mmap")).unwrap()["errno"], 12);
            assert_policy(
                &worker.send(&Value::Null).unwrap(),
                worker.applied_resources(),
            );
            assert!(worker.observed_exit().is_none());
        }
        "inherited_hard_limits" => {
            let mut worker = PythonWorker::launch(
                &config().with_resources(ResourceLimits::new(3, 134_217_728).unwrap()),
            )
            .unwrap();
            let value = worker.send(&json!("descendant")).unwrap();
            assert_policy(&value, worker.applied_resources());
            assert_eq!(value["nnp"], "1");
        }
        "cannot_raise_limits" => {
            let mut worker = PythonWorker::launch(&config()).unwrap();
            let value = worker.send(&json!("raise")).unwrap();
            assert_eq!(value["denied"], json!([true, true]));
            assert_policy(&value, worker.applied_resources());
        }
        "missing_launcher" => {
            let cfg = WorkerConfig::new("python3", "must-not-run.py");
            assert!(matches!(
                PythonWorker::launch(&cfg),
                Err(WorkerError::InvalidConfig(_))
            ));
            assert!(
                cfg.clone()
                    .with_resource_launcher("relative-helper")
                    .is_err()
            );
            let cfg = cfg.with_resource_launcher(temp("missing")).unwrap();
            assert!(matches!(
                PythonWorker::launch(&cfg),
                Err(WorkerError::Launch(_))
            ));
        }
        "setup_failure_precedes_worker" => {
            let wrapper = temp("stricter-launcher");
            let marker = temp("must-not-run");
            script(
                &wrapper,
                &format!(
                    "#!/usr/bin/python3\nimport os,resource,sys\nresource.setrlimit(resource.RLIMIT_AS,(33554432,33554432))\nos.execv({LAUNCHER:?},[{LAUNCHER:?}]+sys.argv[1:])\n"
                ),
            );
            let cfg = config()
                .with_resource_launcher(&wrapper)
                .unwrap()
                .with_arg(marker.to_string_lossy());
            for _ in 0..3 {
                let start = Instant::now();
                let error = match PythonWorker::launch(&cfg) {
                    Ok(_) => panic!("unenforced worker launched"),
                    Err(error) => error,
                };
                assert!(
                    matches!(
                        cause(&error),
                        WorkerError::ResourceSetup {
                            stage: ResourceStage::AddressSpace,
                            errno: Some(1)
                        }
                    ),
                    "{error:?}"
                );
                bounded(start, Duration::from_secs(1));
                assert!(!marker.exists());
            }
            std::fs::remove_file(wrapper).unwrap();
        }
        "bootstrap_deadline_and_storage" => {
            for mode in ["hang", "oversize", "mismatch"] {
                let wrapper = temp(mode);
                let code = match mode {
                    "oversize" => "os.write(1,b'x'*4096)",
                    "mismatch" => "os.write(1,b'{\"kind\":\"limits-v1\",\"cpu\":1,\"as\":1}\\n')",
                    _ => "pass",
                };
                script(
                    &wrapper,
                    &format!(
                        "#!/usr/bin/python3\nimport os,signal,time\nsignal.alarm(5)\n{code}\ntime.sleep(5)\n"
                    ),
                );
                let cfg = config()
                    .with_resource_launcher(&wrapper)
                    .unwrap()
                    .with_limits(IoLimits::new(64, 64, 64, 1).unwrap())
                    .with_timeouts(
                        Timeouts::new(
                            Duration::from_millis(300),
                            Duration::from_secs(1),
                            Duration::from_millis(300),
                            RESERVE,
                        )
                        .unwrap(),
                    );
                let start = Instant::now();
                let error = match PythonWorker::launch(&cfg) {
                    Ok(_) => panic!("bad bootstrap accepted"),
                    Err(error) => error,
                };
                match mode {
                    "hang" => assert!(matches!(cause(&error), WorkerError::HandshakeTimeout)),
                    "oversize" => assert!(matches!(
                        cause(&error),
                        WorkerError::IncomingTooLarge { limit: 64 }
                    )),
                    _ => assert!(matches!(
                        cause(&error),
                        WorkerError::ResourceSetup {
                            stage: ResourceStage::Protocol,
                            ..
                        }
                    )),
                }
                bounded(start, Duration::from_millis(300));
                std::fs::remove_file(wrapper).unwrap();
            }
        }
        "failed_setup_restart_budget" => {
            let wrapper = temp("retry-launcher");
            let counter = temp("counter");
            script(
                &wrapper,
                &format!(
                    "#!/usr/bin/python3\nimport os,resource,sys\nfrom pathlib import Path\np=Path({:?})\nn=int(p.read_text()) if p.exists() else 0\np.write_text(str(n+1))\nif n: resource.setrlimit(resource.RLIMIT_AS,(33554432,33554432))\nos.execv({LAUNCHER:?},[{LAUNCHER:?}]+sys.argv[1:])\n",
                    counter.to_string_lossy()
                ),
            );
            let mut supervisor =
                WorkerSupervisor::start(config().with_resource_launcher(&wrapper).unwrap(), 2)
                    .unwrap();
            supervisor.worker().unwrap().send(&json!("exit")).unwrap();
            await_exit(supervisor.worker().unwrap().process_id());
            assert_eq!(supervisor.health(), HealthState::Unavailable);
            for used in 1..=2 {
                assert!(matches!(
                    cause(&supervisor.poll().unwrap_err()),
                    WorkerError::ResourceSetup {
                        stage: ResourceStage::AddressSpace,
                        ..
                    }
                ));
                assert_eq!(supervisor.restarts_used(), used);
                assert_eq!(supervisor.last_failure(), Some(WorkerFailure::Launch));
            }
            assert!(matches!(
                supervisor.poll(),
                Err(WorkerError::RestartBudgetExhausted { used: 2, max: 2 })
            ));
            assert_eq!(std::fs::read_to_string(&counter).unwrap(), "3");
            std::fs::remove_file(wrapper).unwrap();
            std::fs::remove_file(counter).unwrap();
        }
        "cpu_failure_restart_budget" => {
            let limits = ResourceLimits::new(1, 268_435_456).unwrap();
            let mut supervisor = WorkerSupervisor::start(config().with_resources(limits), 1).unwrap();
            for used in 0..=1 {
                assert_eq!(supervisor.restarts_used(), used);
                let worker = supervisor.worker().unwrap();
                assert_eq!(worker.applied_resources(), limits);
                worker.send(&json!("cpu")).unwrap();
                await_exit(worker.process_id());
                assert_eq!(supervisor.health(), HealthState::Unavailable);
                if used == 0 {
                    assert_eq!(supervisor.poll().unwrap(), HealthState::Unknown);
                }
            }
            assert!(matches!(
                supervisor.poll(),
                Err(WorkerError::RestartBudgetExhausted { used: 1, max: 1 })
            ));
            assert_eq!(supervisor.last_failure(), Some(WorkerFailure::Exited));
        }
        "generic_exit_is_not_exhaustion" => {
            for action in ["exit", "signal"] {
                let mut worker = PythonWorker::launch(&config()).unwrap();
                worker.send(&json!(action)).unwrap();
                await_exit(worker.process_id());
                assert_eq!(worker.health(), HealthState::Unavailable);
                let expected = if action == "exit" {
                    ObservedExit::Code(42)
                } else {
                    ObservedExit::Signal {
                        signal: 9,
                        core_dumped: false,
                    }
                };
                assert_eq!(worker.observed_exit(), Some(expected));
                assert_eq!(worker.last_failure(), Some(WorkerFailure::Exited));
            }
        }
        "privileged_launcher_rejected" => {
            // Required Linux CI negative test; absence of sudo is a failure,
            // never a silent skip. No untrusted target receives a start ack.
            let output = Command::new("sudo")
                .args(["-n", LAUNCHER, "300", "268435456", "--", "/bin/true"])
                .stdin(Stdio::null())
                .output()
                .expect("sudo required for privilege rejection test");
            assert!(!output.status.success());
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(value["kind"], "limitError");
            assert_eq!(value["stage"], "privs");
            assert!(output.stdout.len() <= 64);
        }
        "worker_cannot_reconfigure" => {
            let mut worker = PythonWorker::launch(
                &config().with_config(json!({"cpu": 86400, "as": 1099511627776u64})),
            )
            .unwrap();
            let installed = worker.applied_resources();
            assert_policy(&worker.send(&json!("forge-setup")).unwrap(), installed);
            assert_eq!(worker.applied_resources(), installed);
            let environment = worker.send(&json!("environment")).unwrap();
            assert_eq!(environment["LD_PRELOAD"], Value::Null);
            assert_eq!(environment["LD_LIBRARY_PATH"], Value::Null);
            assert_eq!(environment["HOME"], Value::Null);
        }
        "local_control_during_cpu_exhaustion" => local_control(),
        other => panic!("unknown resource scenario {other}"),
    }
}

fn local_control() {
    use neuradix_safety::{
        AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest, Constraint,
        Generation, Identity, LeaseTable, SafetyGate, SessionConfig, SharedTimeline,
    };
    use neuradix_time::{ClockDomain, Duration as RobotDuration, Timestamp};
    let t = |n| Timestamp::new(ClockDomain::Monotonic, n);
    let generation = Generation::new(1).unwrap();
    let policy = CommandPolicy::new(
        SharedTimeline::new(1, ClockDomain::Monotonic).unwrap(),
        RobotDuration::from_secs(1),
        RobotDuration::ZERO,
        RobotDuration::from_millis(100),
    )
    .unwrap();
    let session = SessionConfig::new(generation, t(0), t(10_000_000_000), policy).unwrap();
    let mut table = LeaseTable::new();
    table
        .grant(AuthorityLease::new(
            Identity::new("local"),
            Capability::new("thrust"),
            session,
            None,
        ))
        .unwrap();
    let mut gate = SafetyGate::new(
        table,
        vec![Constraint::range("range", -1.0, 1.0).unwrap()],
        0.0,
    )
    .unwrap();
    let mut worker = PythonWorker::launch(
        &config().with_resources(ResourceLimits::new(1, 268_435_456).unwrap()),
    )
    .unwrap();
    worker.send(&json!("cpu")).unwrap();
    let task = std::thread::spawn(move || {
        await_exit(worker.process_id());
        assert_eq!(worker.health(), HealthState::Unavailable);
        worker.observed_exit()
    });
    let mut ticks = 0u64;
    while !task.is_finished() {
        ticks += 1;
        let now = t(i128::from(ticks) * 1_000_000);
        let meta = CommandMeta {
            generation,
            sequence: ticks,
            timeline: 1,
            source_at: now,
            deadline: t(now.as_nanos() + 100_000_000),
        };
        let request =
            CommandRequest::new(Identity::new("local"), Capability::new("thrust"), 0.5, meta);
        assert_eq!(gate.evaluate(Some(request), now).applied, 0.5);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        ticks > 10,
        "local control did not progress during CPU exhaustion"
    );
    assert_eq!(
        task.join().unwrap(),
        Some(ObservedExit::Signal {
            signal: 9,
            core_dumped: false
        })
    );
    gate.leases_mut()
        .revoke(&Identity::new("local"), &Capability::new("thrust"));
    let now = t(i128::from(ticks + 1) * 1_000_000);
    assert_eq!(gate.evaluate(None, now).applied, 0.0);
}
