//! Trusted, separately executed Linux resource boundary. Never invoke resource
//! mutation in the multithreaded supervisor or an unsafe pre_exec callback.
#![forbid(unsafe_code)]

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
mod linux {
    use neuradix_python::{ResourceLimits, ResourceStage};
    use nix::sys::prctl::set_no_new_privs;
    use nix::sys::resource::{Resource, getrlimit, rlim_t, setrlimit};
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, ExitCode};

    const ACK: &[u8] = b"{\"kind\":\"start\"}\n";
    type Failure = (ResourceStage, Option<i32>);

    fn acknowledgement() -> std::io::Result<()> {
        let mut input = [0; ACK.len()];
        std::io::stdin().read_exact(&mut input)?;
        if input.as_slice() != ACK {
            return Err(std::io::Error::other("invalid launch acknowledgement"));
        }
        Ok(())
    }
    fn report_error((stage, errno): Failure) -> ExitCode {
        let line = format!(
            "{{\"kind\":\"limitError\",\"stage\":\"{}\",\"errno\":{}}}\n",
            stage.as_str(),
            errno.map_or_else(|| "null".into(), |code| code.to_string())
        );
        // Fits the minimum incoming line. Await ack/EOF to keep a diagnostic
        // waitable until its supervisor consumes it; no worker is ever executed.
        if line.len() <= 64 && std::io::stdout().write_all(line.as_bytes()).is_ok() {
            let _ = acknowledgement();
        }
        ExitCode::from(125)
    }
    fn unprivileged() -> Result<(), Failure> {
        let failure = || (ResourceStage::Privileges, None);
        let file = std::fs::File::open("/proc/self/status")
            .map_err(|error| (ResourceStage::Privileges, error.raw_os_error()))?;
        let mut text = String::new();
        file.take(16_385)
            .read_to_string(&mut text)
            .map_err(|error| (ResourceStage::Privileges, error.raw_os_error()))?;
        if text.len() > 16_384 {
            return Err(failure());
        }
        let mut seen = 0u8;
        for line in text.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            match name {
                "Uid" => {
                    let ids: Vec<u32> = value
                        .split_whitespace()
                        .map(str::parse)
                        .collect::<Result<_, _>>()
                        .map_err(|_| failure())?;
                    if ids.len() != 4 || ids.contains(&0) {
                        return Err(failure());
                    }
                    seen |= 1;
                }
                "CapInh" | "CapPrm" | "CapEff" | "CapAmb" => {
                    if u64::from_str_radix(value.trim(), 16).map_err(|_| failure())? != 0 {
                        return Err(failure());
                    }
                    seen |= match name {
                        "CapInh" => 2,
                        "CapPrm" => 4,
                        "CapEff" => 8,
                        _ => 16,
                    };
                }
                _ => {}
            }
        }
        if seen != 31 {
            return Err(failure());
        }
        Ok(())
    }
    // rlim_t differs across supported Linux libc/word-size combinations.
    #[allow(clippy::useless_conversion)]
    fn install(limits: ResourceLimits) -> Result<(), Failure> {
        unprivileged()?;
        set_no_new_privs().map_err(|error| (ResourceStage::NoNewPrivileges, Some(error as i32)))?;
        let cpu: rlim_t = limits
            .cpu_seconds()
            .try_into()
            .map_err(|_| (ResourceStage::Platform, None))?;
        let memory: rlim_t = limits
            .address_space_bytes()
            .try_into()
            .map_err(|_| (ResourceStage::Platform, None))?;
        for (resource, value, stage) in [
            (Resource::RLIMIT_CPU, cpu, ResourceStage::Cpu),
            (Resource::RLIMIT_AS, memory, ResourceStage::AddressSpace),
        ] {
            setrlimit(resource, value, value).map_err(|error| (stage, Some(error as i32)))?;
            if getrlimit(resource)
                .map_err(|error| (ResourceStage::Verification, Some(error as i32)))?
                != (value, value)
            {
                return Err((ResourceStage::Verification, None));
            }
        }
        Ok(())
    }
    fn prepare() -> Result<(Command, ResourceLimits), Failure> {
        let mut args = std::env::args_os().skip(1);
        let mut number = || {
            args.next()
                .and_then(|value| value.to_str().and_then(|s| s.parse::<u64>().ok()))
                .ok_or((ResourceStage::Configuration, None))
        };
        let cpu = number()?;
        let bytes = number()?;
        if args.next() != Some(OsString::from("--")) {
            return Err((ResourceStage::Configuration, None));
        }
        let program = args.next().ok_or((ResourceStage::Configuration, None))?;
        let limits = ResourceLimits::new(cpu, bytes)
            .and_then(ResourceLimits::for_current_platform)
            .map_err(|_| (ResourceStage::Configuration, None))?;
        let mut command = Command::new(program);
        command.args(args);
        Ok((command, limits))
    }
    pub fn run() -> ExitCode {
        let (mut command, limits) = match prepare() {
            Ok(value) => value,
            Err(error) => return report_error(error),
        };
        // Prepare all ordinary allocations before lowering address space. Failure
        // to report/exec afterwards still cannot run the target without limits.
        let confirmation = format!(
            "{{\"kind\":\"limits-v1\",\"cpu\":{},\"as\":{}}}\n",
            limits.cpu_seconds(),
            limits.address_space_bytes()
        );
        if confirmation.len() > 64 {
            return report_error((ResourceStage::Protocol, None));
        }
        if let Err(error) = install(limits) {
            return report_error(error);
        }
        if std::io::stdout()
            .write_all(confirmation.as_bytes())
            .is_err()
            || acknowledgement().is_err()
        {
            return ExitCode::from(125);
        }
        let error = command.exec();
        // After ack, stdout may belong to untrusted worker code. Do not mint
        // further trusted setup reports in that phase; report an ordinary exit.
        eprintln!("resource launcher could not exec interpreter: {error}");
        ExitCode::from(126)
    }
}

fn main() -> std::process::ExitCode {
    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    {
        linux::run()
    }
    #[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
    {
        eprintln!("bounded worker resource launch is unsupported on this platform");
        std::process::ExitCode::FAILURE
    }
}
