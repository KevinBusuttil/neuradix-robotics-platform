//! Host trusted startup with a durable generation reservation (WP-A04.4).
//!
//! A Unix walk-through with one host test driver; no physical device is operated
//! or qualified:
//!
//! 1. a fresh, unique, owner-only (0700) state directory per (receiver, binding);
//! 2. boot 0 over the blank store fails closed: the adapter only writes its safe
//!    output, for a bounded number of ticks;
//! 3. explicit, out-of-band provisioning with a registry epoch (the
//!    `provisioning` feature; enabled here through the dev-dependency, never in
//!    an actuator build);
//! 4. boot 1 reserves a generation (durable in both slot files) before a strict
//!    grant, then runs;
//! 5. boot 2 restarts with its clock back at 0 and opens with
//!    `RollbackDefense::Witness(g1)`;
//! 6. a replayed g1 command with fresh boot-2 timestamps is rejected;
//! 7. the state directory is removed.
//!
//! Every error after the adapter exists routes to one bounded safe-tick loop; no
//! `?` returns past a live adapter. A real process keeps ticking there until
//! trusted provisioning or maintenance; this example stops after a few ticks.

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    host::main()
}

#[cfg(not(unix))]
fn main() {
    println!("reserved_startup: FileReservationStore is available on Unix hosts only");
}

#[cfg(unix)]
mod host {
    use std::fs;
    use std::num::NonZeroU32;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;
    use std::time::{SystemTime, UNIX_EPOCH};

    use neuradix_safety::actuator::{
        ActuatorAdapter, ActuatorBinding, ActuatorConfig, ActuatorDriver, DriverError,
        DriverPermission, ExecutionMode, PermissionError, PermissionStatus,
    };
    use neuradix_safety::reservation::{
        FileReservationStore, FileStoreError, GenerationReserver, NamespaceEpoch, OpenError,
        ProvisionError, ProvisionGuards, ReceiverId, Remedy, ReservationKey, ReserveError,
        ReserverConfig, RollbackDefense, provision,
    };
    use neuradix_safety::{
        AuthorityLease, Capability, CommandMeta, CommandPolicy, CommandRequest, Generation,
        Identity, Outcome, RejectReason, SessionConfig, SessionError, SharedTimeline,
    };
    use neuradix_time::{ClockDomain, Duration, Timestamp};

    const HOLDER: &str = "controller";
    const CAPABILITY: &str = "thrust";
    const DRIVER: &str = "example/thruster";
    /// Receiver identity from host provisioning configuration; never stored in
    /// (or learned from) the state directory.
    const RECEIVER: [u8; 16] = *b"example-host-001";
    /// Generations covered by one durable commit.
    const WINDOW: u32 = 4;
    /// Bounded fail-closed ticks per failed boot (a real process keeps ticking).
    const SAFE_TICKS: usize = 3;
    const SAFE_OUTPUT: f64 = 0.0;

    /// One host test driver: records writes, never fails.
    #[derive(Default)]
    struct ExampleDriver {
        writes: usize,
    }
    impl ActuatorDriver for ExampleDriver {
        fn endpoint(&self) -> &str {
            DRIVER
        }
        fn mode(&self) -> ExecutionMode {
            ExecutionMode::Live
        }
        fn write(&mut self, value: f64) -> Result<(), DriverError> {
            self.writes += 1;
            println!("    driver write #{}: {value}", self.writes);
            Ok(())
        }
    }

    type Adapter = ActuatorAdapter<ExampleDriver>;
    type Reserver = GenerationReserver<FileReservationStore>;

    /// Why a boot failed closed. Never holds the store.
    enum Startup {
        Store(FileStoreError),
        Open(OpenError),
        Reserve(ReserveError),
        Session(SessionError),
        Permission(PermissionError),
        /// The walk-through observed something other than the documented outcome.
        Unexpected(&'static str),
    }
    impl Startup {
        fn remedy(&self) -> Option<Remedy> {
            match self {
                Self::Open(e) => Some(e.remedy()),
                Self::Reserve(e) => Some(e.remedy()),
                _ => None,
            }
        }
    }
    impl std::fmt::Display for Startup {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Store(e) => write!(f, "state directory refused: {e}"),
                Self::Open(e) => write!(f, "reservation open refused: {e:?}"),
                Self::Reserve(e) => write!(f, "reservation failed: {e:?}"),
                Self::Session(e) => write!(f, "lease configuration: {e:?}"),
                Self::Permission(e) => write!(f, "permission refused: {e:?}"),
                Self::Unexpected(what) => write!(f, "unexpected outcome: {what}"),
            }
        }
    }
    impl From<FileStoreError> for Startup {
        fn from(e: FileStoreError) -> Self {
            Self::Store(e)
        }
    }
    impl From<ReserveError> for Startup {
        fn from(e: ReserveError) -> Self {
            Self::Reserve(e)
        }
    }
    impl From<SessionError> for Startup {
        fn from(e: SessionError) -> Self {
            Self::Session(e)
        }
    }
    impl From<PermissionError> for Startup {
        fn from(e: PermissionError) -> Self {
            Self::Permission(e)
        }
    }

    fn check(condition: bool, what: &'static str) -> Result<(), Startup> {
        if condition {
            Ok(())
        } else {
            Err(Startup::Unexpected(what))
        }
    }

    /// Simulated monotonic clock; every boot restarts it at 0.
    struct Clock {
        ms: i128,
    }
    impl Clock {
        fn now(&self) -> Timestamp {
            Timestamp::new(ClockDomain::Monotonic, self.ms * 1_000_000)
        }
        fn after(&self, ms: i128) -> Timestamp {
            Timestamp::new(ClockDomain::Monotonic, (self.ms + ms) * 1_000_000)
        }
        fn advance(&mut self, ms: i128) -> Timestamp {
            self.ms += ms;
            self.now()
        }
    }

    /// The unique scratch root, removed on drop even if the walk-through fails.
    struct StateRoot(PathBuf);
    impl Drop for StateRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// What trusted provisioning tooling does: an existing owner-only directory.
    fn owner_only_dir(path: &Path) -> std::io::Result<()> {
        fs::DirBuilder::new().mode(0o700).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }

    fn policy() -> Result<CommandPolicy, SessionError> {
        CommandPolicy::new(
            SharedTimeline::new(1, ClockDomain::Monotonic)?,
            Duration::from_millis(500),
            Duration::ZERO,
            Duration::from_millis(1_000),
        )
    }

    fn command(g: Generation, sequence: u64, at: Timestamp, value: f64) -> CommandRequest {
        CommandRequest::new(
            Identity::new(HOLDER),
            Capability::new(CAPABILITY),
            value,
            CommandMeta {
                generation: g,
                sequence,
                source_at: at,
                deadline: Timestamp::new(at.domain(), at.as_nanos() + 500_000_000),
                timeline: 1,
            },
        )
    }

    /// Every startup, after `new_reserved` and a safe tick: open with an explicit
    /// rollback posture (logged), reserve (the first reserve always commits),
    /// bind the token to a lease, strict grant. Returns the reserver for spares.
    fn start(
        adapter: &mut Adapter,
        binding: &ActuatorBinding,
        key: ReservationKey,
        state_dir: &Path,
        rollback: RollbackDefense,
        clock: &Clock,
    ) -> Result<(Reserver, Generation), Startup> {
        let store = FileReservationStore::open(state_dir)?;
        let window = NonZeroU32::new(WINDOW).ok_or(Startup::Unexpected("window"))?;
        let mut reserver =
            GenerationReserver::open(store, key, ReserverConfig::new(window, rollback))
                .map_err(|f| Startup::Open(f.error))?;
        println!("  rollback posture: {:?}", reserver.status().rollback);
        let token = reserver.reserve()?; // durable in both slot files before activation
        let g = token.generation();
        println!("  reserved generation {:#x}", g.get());
        let session = SessionConfig::new(g, clock.now(), clock.after(60_000), policy()?)?;
        let lease = AuthorityLease::new(
            Identity::new(HOLDER),
            Capability::new(CAPABILITY),
            session,
            None,
        );
        let report = adapter.grant(
            DriverPermission::reserved(binding.clone(), lease, token)?,
            clock.now(),
        )?;
        check(
            report.permission == PermissionStatus::Initialized && report.output == SAFE_OUTPUT,
            "grant establishes the safe output",
        )?;
        Ok((reserver, g))
    }

    /// The single fail-closed path: never grant, keep writing only the safe
    /// output, report the remedy. Bounded here; a real process keeps ticking.
    fn fail_closed(adapter: &mut Adapter, clock: &mut Clock, error: &Startup) -> bool {
        println!(
            "  startup failed closed: {error}; remedy {:?}",
            error.remedy()
        );
        let mut safe = true;
        for _ in 0..SAFE_TICKS {
            let now = clock.advance(100);
            let report = adapter.port(ExecutionMode::Live, now).tick(None);
            println!(
                "  safe tick: permission {:?}, output {}",
                report.permission, report.output
            );
            safe &= report.output == SAFE_OUTPUT && report.write_result == Some(Ok(()));
        }
        adapter.shutdown(clock.now());
        safe
    }

    /// Boot-specific work while granted (commands, replacement, replay checks).
    type Session<'a> = &'a mut dyn FnMut(
        &mut Adapter,
        &mut Reserver,
        &mut Clock,
        Generation,
    ) -> Result<(), Startup>;

    enum Boot {
        Ran(Generation),
        FailedClosed { error: Startup, safe: bool },
    }

    /// One process lifetime: construct, tick safe, start, run or fail closed.
    fn boot(
        binding: &ActuatorBinding,
        key: ReservationKey,
        state_dir: &Path,
        rollback: RollbackDefense,
        session: Session<'_>,
    ) -> Result<Boot, PermissionError> {
        let mut clock = Clock { ms: 0 };
        let config = ActuatorConfig::new(binding.clone(), -1.0, 1.0, 10.0, SAFE_OUTPUT)?;
        // Construction failures own nothing, so there is nothing to tick.
        let mut adapter = ActuatorAdapter::new_reserved(config, ExampleDriver::default())?;
        let report = adapter.port(ExecutionMode::Live, clock.now()).tick(None);
        println!(
            "  before reservation: permission {:?}, output {}",
            report.permission, report.output
        );
        let started = start(&mut adapter, binding, key, state_dir, rollback, &clock).and_then(
            |(mut reserver, g)| {
                session(&mut adapter, &mut reserver, &mut clock, g)?;
                Ok(g)
            },
        );
        Ok(match started {
            Ok(g) => {
                adapter.shutdown(clock.advance(10));
                Boot::Ran(g)
            }
            Err(error) => {
                let safe = fail_closed(&mut adapter, &mut clock, &error);
                Boot::FailedClosed { error, safe }
            }
        })
    }

    /// Trusted, out-of-band provisioning tool (never the boot path): the epoch
    /// comes from a durable fleet registry and the floor is advanced first.
    fn trusted_provisioning_tool(
        state_dir: &Path,
        key: ReservationKey,
        registry_epoch: NamespaceEpoch,
    ) -> Result<(), String> {
        let mut store = FileReservationStore::open(state_dir).map_err(|e| e.to_string())?;
        let guards = ProvisionGuards {
            floor: Some(registry_epoch),
            prior_high_water: None,
        };
        let report = provision(&mut store, key, registry_epoch, guards)
            .map_err(|e: ProvisionError| e.to_string())?;
        println!("  provisioned {report:?}");
        Ok(())
    }

    fn walk_through(root: &Path) -> Result<(), String> {
        let binding = ActuatorBinding::new(HOLDER, CAPABILITY, DRIVER, ExecutionMode::Live)
            .map_err(|e| e.to_string())?;
        let receiver = ReceiverId::new(RECEIVER).ok_or("unprogrammed receiver id")?;
        let key = ReservationKey::new(receiver, binding.reservation_key());
        let epoch_1 = NamespaceEpoch::new(1).ok_or("epoch")?;

        // 1. One existing 0700 directory per (receiver, binding); `open` never creates it.
        let state_dir = root.join(format!("{:016x}", binding.reservation_key().get()));
        owner_only_dir(&state_dir).map_err(|e| e.to_string())?;
        println!("state directory {}", state_dir.display());

        // 2. Boot 0: the blank store fails closed with safe ticks only.
        println!("boot 0 (blank store):");
        let mut no_session = |_: &mut Adapter,
                              _: &mut Reserver,
                              _: &mut Clock,
                              _: Generation|
         -> Result<(), Startup> { Ok(()) };
        match boot(
            &binding,
            key,
            &state_dir,
            RollbackDefense::Floor(epoch_1),
            &mut no_session,
        )
        .map_err(|e| e.to_string())?
        {
            Boot::FailedClosed {
                error: Startup::Open(OpenError::Blank),
                safe: true,
            } => {}
            Boot::FailedClosed { error, safe } => {
                return Err(format!("boot 0: unexpected {error} (safe ticks: {safe})"));
            }
            Boot::Ran(_) => return Err("boot 0 must not grant from a blank store".into()),
        }

        // 3. Explicit, out-of-band provisioning.
        println!("trusted provisioning:");
        trusted_provisioning_tool(&state_dir, key, epoch_1)?;

        // 4. Boot 1: reserve, strict grant, accept commands, take a spare.
        println!("boot 1:");
        let mut boot1 = |adapter: &mut Adapter,
                         reserver: &mut Reserver,
                         clock: &mut Clock,
                         g: Generation|
         -> Result<(), Startup> {
            for sequence in 0..2 {
                let now = clock.advance(100);
                let report = adapter
                    .port(ExecutionMode::Live, now)
                    .tick(Some(command(g, sequence, now, 0.2)));
                let outcome = report.decision.map(|d| d.outcome);
                println!("  command {sequence} with g1: {outcome:?}");
                check(outcome == Some(Outcome::Accepted), "g1 command accepted")?;
            }
            // A spare from the committed window costs no store call; unused, it is burned.
            let spare = reserver.reserve_from_window()?;
            println!(
                "  spare {:#x} (dropped unused: burned, never reissued)",
                spare.generation().get()
            );
            Ok(())
        };
        let g1 = match boot(
            &binding,
            key,
            &state_dir,
            RollbackDefense::Floor(epoch_1),
            &mut boot1,
        )
        .map_err(|e| e.to_string())?
        {
            Boot::Ran(g) => g,
            Boot::FailedClosed { error, .. } => {
                return Err(format!("boot 1 failed closed: {error}"));
            }
        };

        // 5–6. Boot 2: clock back at 0, witness g1 (the highest generation handed
        // to a source, from a trusted record outside the reservation directory).
        println!("boot 2 (restart, witness g1):");
        let mut boot2 = |adapter: &mut Adapter,
                         _: &mut Reserver,
                         clock: &mut Clock,
                         g2: Generation|
         -> Result<(), Startup> {
            check(g2 > g1, "g2 above g1")?;
            let now = clock.advance(100);
            let replay = adapter
                .port(ExecutionMode::Live, now)
                .tick(Some(command(g1, 2, now, 0.9)));
            let outcome = replay.decision.map(|d| d.outcome);
            println!(
                "  replayed g1 command: {outcome:?}, output {}",
                replay.output
            );
            check(
                outcome == Some(Outcome::Rejected(RejectReason::GenerationMismatch))
                    && replay.output == SAFE_OUTPUT,
                "replayed g1 rejected with the safe output",
            )?;
            let now = clock.advance(100);
            let report = adapter
                .port(ExecutionMode::Live, now)
                .tick(Some(command(g2, 0, now, 0.2)));
            let outcome = report.decision.map(|d| d.outcome);
            println!("  command 0 with g2: {outcome:?}");
            check(outcome == Some(Outcome::Accepted), "g2 command accepted")
        };
        match boot(
            &binding,
            key,
            &state_dir,
            RollbackDefense::Witness(g1),
            &mut boot2,
        )
        .map_err(|e| e.to_string())?
        {
            Boot::Ran(g2) => println!("  g1 {:#x} < g2 {:#x}", g1.get(), g2.get()),
            Boot::FailedClosed { error, .. } => {
                return Err(format!("boot 2 failed closed: {error}"));
            }
        }
        Ok(())
    }

    pub fn main() -> ExitCode {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let root = StateRoot(std::env::temp_dir().join(format!(
            "neuradix-reserved-startup-{}-{nanos}",
            std::process::id()
        )));
        if let Err(e) = owner_only_dir(&root.0) {
            eprintln!("cannot create {}: {e}", root.0.display());
            return ExitCode::FAILURE;
        }
        let result = walk_through(&root.0);
        // 7. Remove the directory.
        let removed = fs::remove_dir_all(&root.0).is_ok() && !root.0.exists();
        drop(root);
        match result {
            Ok(()) if removed => {
                println!("reserved startup verified; state directory removed");
                ExitCode::SUCCESS
            }
            Ok(()) => {
                eprintln!("state directory was not removed");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("reserved startup walk-through failed: {e}");
                ExitCode::FAILURE
            }
        }
    }
}
