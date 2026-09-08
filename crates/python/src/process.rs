//! Linux nonblocking stdio, owned process groups and bounded deferred reaping.

use std::time::{Duration, Instant};

/// Result of the one bounded cleanup attempt for a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupState {
    /// The direct child was reaped; its group received the cleanup signal.
    Reaped,
    /// The direct child occupies a bounded deferred-reaper slot.
    Deferred,
    /// An external reaper took ownership; no potentially reused PID was signalled.
    OwnershipLost,
    /// The reaper failed; the admission slot is permanently withheld.
    ReaperUnavailable,
}

/// Auditable cleanup result. Group signalling is not proof that all descendants
/// have exited; escaped sessions and arbitrary resource containment are deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupReport {
    /// Direct child process ID, for diagnosis.
    pub pid: u32,
    /// Reaping outcome.
    pub state: CleanupState,
    /// OS error from group signalling, if any (ESRCH means already gone).
    pub signal_error: Option<i32>,
}

pub(crate) fn pause_until(deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() { std::thread::sleep(remaining.min(Duration::from_millis(1))); }
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
mod linux {
    use std::io::{self, Read, Write};
    use std::os::fd::AsFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, ChildStdin, ChildStdout, Command};
    use std::sync::mpsc::{self, Receiver, SyncSender};
    use std::sync::{LazyLock, Mutex};
    use std::time::{Duration, Instant};

    use nix::errno::Errno;
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use nix::sys::signal::{Signal, killpg};
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
    use nix::unistd::Pid;

    use super::{CleanupReport, CleanupState, pause_until};
    use crate::WorkerError;

    // Live children and deferred reaping share the same admission budget.
    const PROCESS_SLOTS: usize = 32;
    struct Reaper {
        available: Mutex<Receiver<()>>,
        release: SyncSender<()>,
        pending: SyncSender<Child>,
    }
    static REAPER: LazyLock<io::Result<Reaper>> = LazyLock::new(|| {
        let (release, available) = mpsc::sync_channel(PROCESS_SLOTS);
        let (pending, rx) = mpsc::sync_channel::<Child>(PROCESS_SLOTS);
        for _ in 0..PROCESS_SLOTS { release.try_send(()).expect("empty admission queue"); }
        let free = release.clone();
        std::thread::Builder::new().name("neuradix-child-reaper".into()).spawn(move || {
            let mut children = Vec::<Child>::with_capacity(PROCESS_SLOTS);
            loop {
                match rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(child) => children.push(child),
                    Err(mpsc::RecvTimeoutError::Timeout) => {},
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
                children.retain_mut(|child| match child.try_wait() {
                    Ok(Some(_)) => { let _ = free.try_send(()); false },
                    // External reaping violates ownership, but do not leak a slot.
                    Err(e) if e.raw_os_error() == Some(Errno::ECHILD as i32) => { let _ = free.try_send(()); false },
                    _ => true,
                });
            }
        })?;
        Ok(Reaper { available: Mutex::new(available), release, pending })
    });

    struct Permit { active: bool }
    impl Permit {
        fn acquire() -> Result<Self, WorkerError> {
            let reaper = REAPER.as_ref().map_err(|e| WorkerError::Io(io::Error::new(e.kind(), "cannot start bounded child reaper")))?;
            let queue = reaper.available.try_lock().map_err(|_| WorkerError::ProcessCapacity)?;
            queue.try_recv().map_err(|_| WorkerError::ProcessCapacity)?;
            Ok(Self { active: true })
        }
        fn defer(mut self, child: Child) -> CleanupState {
            // The token remains reserved until the reaper observes actual exit.
            // With 32 shared tokens a 32-entry channel cannot overflow normally.
            self.active = false;
            match REAPER.as_ref().expect("admitted reaper").pending.try_send(child) {
                Ok(()) => CleanupState::Deferred,
                Err(_) => CleanupState::ReaperUnavailable,
            }
        }
    }
    impl Drop for Permit {
        fn drop(&mut self) {
            if self.active { let _ = REAPER.as_ref().expect("admitted reaper").release.try_send(()); }
        }
    }

    fn nonblocking(fd: &impl AsFd) -> io::Result<()> {
        let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
        fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
        Ok(())
    }

    pub(crate) struct Process {
        child: Option<Child>,
        stdin: Option<ChildStdin>,
        stdout: Option<ChildStdout>,
        pid: Pid,
        permit: Option<Permit>,
        report: Option<CleanupReport>,
        owns_child: bool,
    }
    impl Process {
        pub fn spawn(command: &mut Command, cleanup_end: Instant) -> Result<Self, WorkerError> {
            let permit = Permit::acquire()?;
            let mut child = command.process_group(0).spawn().map_err(WorkerError::Launch)?;
            let pid = Pid::from_raw(child.id() as i32);
            let stdin = child.stdin.take();
            let stdout = child.stdout.take();
            let mut process = Self { child: Some(child), stdin, stdout, pid, permit: Some(permit), report: None, owns_child: true };
            let setup = (|| {
                nonblocking(process.stdin.as_ref().ok_or_else(|| io::Error::other("missing stdin"))?)?;
                nonblocking(process.stdout.as_ref().ok_or_else(|| io::Error::other("missing stdout"))?)
            })();
            if let Err(error) = setup {
                let cleanup = process.finish(cleanup_end);
                return Err(WorkerError::Cleanup { cause: Box::new(WorkerError::Io(error)), report: cleanup });
            }
            Ok(process)
        }
        pub fn id(&self) -> u32 { self.pid.as_raw() as u32 }
        pub fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.stdout.as_mut().ok_or_else(|| io::Error::other("closed stdout"))?.read(bytes)
        }
        pub fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.stdin.as_mut().ok_or_else(|| io::Error::other("closed stdin"))?.write(bytes)
        }
        pub fn close_stdin(&mut self) { self.stdin.take(); }
        pub fn has_exited(&mut self) -> Result<bool, WorkerError> {
            if self.report.is_some() { return Ok(true); }
            // Keep the leader waitable until killpg has run: reaping it earlier
            // would let its PID/PGID be reused before group cleanup.
            match waitid(Id::Pid(self.pid), WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT) {
                Ok(WaitStatus::StillAlive) => Ok(false),
                Ok(_) => Ok(true),
                Err(Errno::EINTR) => Ok(false),
                Err(Errno::ECHILD) => { self.owns_child = false; Err(WorkerError::ProcessOwnershipLost) },
                Err(error) => Err(WorkerError::Io(error.into())),
            }
        }
        pub fn finish(&mut self, deadline: Instant) -> CleanupReport {
            if let Some(report) = self.report { return report; }
            self.stdin.take(); self.stdout.take();
            let _ = self.has_exited();
            let mut signal_error = None;
            let state = if !self.owns_child {
                self.child.take(); self.permit.take();
                CleanupState::OwnershipLost
            } else {
                if let Err(error) = killpg(self.pid, Signal::SIGKILL) {
                    if error != Errno::ESRCH { signal_error = Some(error as i32); }
                }
                let mut child = self.child.take().expect("owned child");
                let reaped = loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break true,
                        Err(error) if error.raw_os_error() == Some(Errno::ECHILD as i32) => break true,
                        _ => {},
                    }
                    if Instant::now() >= deadline { break false; }
                    pause_until(deadline);
                };
                let permit = self.permit.take().expect("owned admission");
                if reaped { drop(permit); CleanupState::Reaped } else { permit.defer(child) }
            };
            let report = CleanupReport { pid: self.id(), state, signal_error };
            self.report = Some(report);
            report
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            // Emergency/unwind path performs no timed wait and never sends a
            // second signal after an earlier cleanup (avoids PID reuse).
            if self.report.is_none() { self.finish(Instant::now()); }
        }
    }
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
pub(crate) use linux::Process;

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
mod unsupported {
    use std::{io, process::Command, time::Instant};
    use super::{CleanupReport, CleanupState};
    use crate::WorkerError;
    pub(crate) struct Process;
    impl Process {
        pub fn spawn(_: &mut Command, _: Instant) -> Result<Self, WorkerError> { Err(WorkerError::UnsupportedPlatform) }
        pub fn id(&self) -> u32 { 0 }
        pub fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Err(io::ErrorKind::Unsupported.into()) }
        pub fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::ErrorKind::Unsupported.into()) }
        pub fn close_stdin(&mut self) {}
        pub fn has_exited(&mut self) -> Result<bool, WorkerError> { Err(WorkerError::UnsupportedPlatform) }
        pub fn finish(&mut self, _: Instant) -> CleanupReport { CleanupReport { pid: 0, state: CleanupState::OwnershipLost, signal_error: None } }
    }
}
#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
pub(crate) use unsupported::Process;
