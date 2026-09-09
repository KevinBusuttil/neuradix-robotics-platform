# WP-A05: Linux per-process CPU-time and address-space limits

This bounded increment belongs to **WP-A05: Bounded extension processes**.
It adds actual Linux kernel enforcement to the integrated
[I/O](WP-A05-Bounded-Worker-IO.md) and [heartbeat](WP-A05-Worker-Heartbeat.md)
increments. WP-A05/ACC-09 remain **partial**. The resource PR is unmerged until
review and integration; opening it does not complete integration.

## Prerequisite review

[PR #12](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/12)
was already merged as `5cda65a93375899ac148f372b246f36f965857cd` when this task
started. Its current head remained `0cd31b773aca74f5807f8d9d367416206a393535`.
The merge tree exactly matches that head. Code, API migration, heartbeat window
and matching-reply logic, launch accounting and acceptance evidence were reviewed;
no additional concrete prerequisite defect was identified. There were no review
comments, main was unprotected and the ruleset list was empty at inspection.
[PR CI 34320795164](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34320795164)
and [push CI 34320791968](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34320791968)
still passed on that exact revision. This isolated branch starts from updated
main; existing checkouts and branches were preserved. No applicable AGENTS.md
was present. WP-A01's integrated baseline and pinned CI are prerequisites;
its broader evidence inventory and optional-tool audit remain open.

## Defect and policy

Previously an otherwise bounded worker could consume CPU indefinitely or grow
its virtual address space without a supervisor-installed limit. Heartbeats bound
responsiveness windows, not CPU use or allocations. The new private
`ResourceLimits` installs **equal soft and hard** limits in the child only.

| Configuration | Accepted values | Default | Enforced meaning |
|---|---|---|---|
| `cpu_seconds` | Integer 1..=86,400 seconds | 300 seconds | RLIMIT_CPU lifetime process CPU time across all its threads, including launch/interpreter work |
| `address_space_bytes` | Integer 16,777,216..=1,099,511,627,776 bytes | 268,435,456 bytes (256 MiB) | RLIMIT_AS virtual address space per process |

Zero, unlimited sentinels and values beyond these bounds are rejected. Integer
APIs cannot express NaN/infinity/fractional seconds. CPU seconds are not rounded
up from elapsed durations. Address space is rounded **down** to the system page
size; `for_current_platform()` checks native representation and rejects any
value reaching RLIM_INFINITY. Division precedes multiplication, so rounding does
not overflow. Resolution alone does not install limits. The effective initial
policy is available through `PythonWorker::applied_resources()`; workers may
subsequently lower their limits. Legal configuration does not guarantee that a
particular Python interpreter or workload fits, especially at the 16 MiB minimum.

At the CPU hard limit Linux terminates the process with SIGKILL, regardless of
its SIGXCPU handler. CPU time excludes idle sleep and is not a wall-clock timeout.
Address-space denial can yield ENOMEM/MemoryError without killing the process;
a handled matching application error still proves responsiveness. RSS, shared
physical memory and GPU memory are different quantities. Ordinary fork/exec
descendants inherit the limits, but a new child starts separate CPU accounting:
these limits do **not** bound aggregate process-tree consumption.
See the Linux [getrlimit/setrlimit specification](https://man7.org/linux/man-pages/man2/getrlimit.2.html).

## Trusted launch and failure boundary

`neuradix-python-launcher` is a small native executable built from this package.
The supervisor requires its absolute path; it never changes its own limits and
does not run an unsafe `pre_exec` callback. Safe APIs from the already locked
`nix` 0.30.1 dependency install the policy in this separately executed child.
The helper then execs Python in the **same PID and process group**. No extra
worker slot, per-worker thread, outstanding request or background task is added.

The operator must install the matching helper and protect its executable,
libraries and containing directories from worker modification. Selecting an
absolute path is not executable authentication. This is a trusted supervisor
configuration boundary; neither worker payloads nor the SDK can select limits.
Before exec, the supervisor clears the helper environment, retaining only PATH,
LANG=C.UTF-8 and explicit SDK configuration/import-path variables. Ambient
loader hooks, HOME and other environment variables are not forwarded. Interpreter
lookup through PATH and Python imports occur after limits are installed.

The helper reads at most 16 KiB of `/proc/self/status`, requires nonzero
real/effective/saved/filesystem UIDs and zero inheritable/permitted/effective/
ambient capabilities, then installs irreversible `no_new_privs`. Root or
capability-bearing execution, unavailable procfs or failed privilege enforcement
rejects launch. This prevents workers gaining setuid/file-capability privilege
through exec to raise their hard limits; it does not isolate same-user processes
or protect against an external privileged administrator. See Linux
[PR_SET_NO_NEW_PRIVS](https://man7.org/linux/man-pages/man2/PR_SET_NO_NEW_PRIVS.2const.html).

For each resource, the helper installs and reads back both kernel limits.
An inherited hard limit below the requested value causes explicit setup failure;
the helper does not silently choose a different policy or run unrestricted.
A bounded `limits-v1` confirmation contains the exact effective CPU/AS values.
The parent consumes it, checks both values, then sends a fixed `start` acknowledgement.
Only then does the helper exec Python and the ordinary ready handshake begin.
This serial bootstrap works with the minimum 64-byte line/queue and one-message
limits. The helper reads exactly the acknowledgement, without prefetching worker
requests. Bootstrap traffic never establishes heartbeat health.

Before acknowledgement, `ResourceSetup { stage, errno }` exposes bounded setup
diagnostics: configuration, platform, privilege, no-new-privileges, CPU, AS,
verification or protocol. A setup-error helper waits for EOF/ack so its report
can be consumed before cleanup, but never executes the worker. Missing executable
paths return `Launch`; missing configured helper returns `InvalidConfig`.
Timeouts, malformed/oversized confirmations and missing ready all retire and
clean up the child. After acknowledgement stdout can be worker-controlled, so
further setup claims cannot change the policy or become trusted diagnostics.
An interpreter exec failure is an ordinary startup exit/EOF, not inferred from
an otherwise ambiguous exit code.

`ObservedExit` records an actual wait observation before cleanup signalling:
exit code or terminating signal plus core-dump flag. `WorkerExited.status`
includes that observation when available. Generic SIGKILL, SIGSEGV, code 125/126
or a Python MemoryError message does not establish a resource-exhaustion cause.
The controlled CPU test supplies additional experimental evidence; production
health uses the existing `Exited` reason. Setup failure exposes its exact stage;
replacement setup failures retain `WorkerFailure::Launch` and consume the budget.

## Preserved supervision and OS scope

Preparation, spawn, limit confirmation, acknowledgement and ready share the
**original absolute handshake deadline**, including its cleanup reserve.
Deadline equality expires; no bootstrap phase restarts it. A stalled helper or
worker receives the existing bounded process-group cleanup/deferred-reaping
path. All previous framing, serialization, one-outstanding-request, 32-process
admission and single bounded reaper limits remain in force.

Heartbeat due/expiry windows, matching timely replies, latched failures and
periodic idle polling are unchanged. `WorkerSupervisor::poll()` must still run
at/before due outside conventional local-control execution. Replacements get
fresh process CPU accounting and the same trusted resource policy; every attempt,
including failed setup, is charged before launch. Repeated resource failures
cannot bypass restart exhaustion. Shutdown never implicitly relaunches.

The implemented backend targets unprivileged Linux GNU/musl with working procfs;
execution evidence below qualifies only the CI Linux GNU x86_64 environment.
Other OSes/uclibc reject launch. Musl, other architectures and deployment kernels
need separate qualification. OS scheduling, executable/filesystem/syscall latency
and uninterruptible kernel states prevent hard-real-time return guarantees.
Existing cleanup reports retain deferred/unconfirmed outcomes explicitly.
Escaped sessions, aggregate CPU/RSS, GPU enforcement, other OS backends and
deployment supervision remain deferred. Process separation is not a sandbox.

## API and example migration

Build/install the helper from the same locked revision and pass its trusted
absolute path. No Python SDK or worker protocol migration is needed: the helper
consumes the private bootstrap before Python starts. Custom workers still use
the existing bounded ready/request/pong protocol.

```sh
cargo build --locked -p neuradix-python --bin neuradix-python-launcher
cargo run --locked -p neuradix-example-python-worker -- target/debug/neuradix-python-launcher
```

```rust,ignore
let config = WorkerConfig::new("python3", "worker.py")
    .with_resource_launcher("/opt/neuradix/bin/neuradix-python-launcher")?
    .with_resources(ResourceLimits::new(300, 256 * 1024 * 1024)?);
```

`with_resources` accepts only validated values. `WorkerConfig::resources()`
reads requested policy; `PythonWorker::applied_resources()` reads confirmed
page-rounded policy. The example canonicalizes its explicit helper argument
and prints installed limits. Environment-dependent workers must account for the
restricted inherited environment. Tests use Cargo's matching binary path.

## Verification evidence

Current-revision results will be recorded here after pinned CI completes.
Local Rust/Cargo and AVR tools are unavailable. Six SDK tests passed locally
under a 30-second external timeout. The actual kernel enforcement tests require
an unprivileged Linux process, Python 3 and passwordless sudo for the explicit
root-rejection negative test; missing prerequisites fail, never silently skip.
That negative test runs only the trusted helper and never acknowledges target exec.

Resource scenarios have a 20-second external subprocess timeout; hostile Python
fixtures also set a 10-second alarm. Tests allow 300 ms scheduling tolerance over
declared operation budgets. CPU exhaustion is independently observed within five
seconds for a one-CPU-second policy; no API promises this wall-time ratio under
arbitrary host load. Existing I/O/heartbeat subprocess guards and 120/180-second
suite timeouts remain. Focused counts overlap workspace counts; nested helper
test reports must not be counted twice.

The resource tests inspect real `getrlimit`, failed hard-limit raises, descendant
inheritance, `no_new_privs`, CPU SIGKILL and mmap ENOMEM/allocation failure.
They cover exact constructor/page boundaries, minimum one-message bootstrap,
setup failure before worker marker execution, bounded setup traffic/deadlines,
failed-launch and CPU-failure restart exhaustion, generic exits without guessed
causes, forged policy traffic, bounded cleanup and an actual independent local
SafetyGate with revoked-authority fallback while the extension exhausts CPU.
All A04 and previous A05 regressions remain required.

## Remaining acceptance

Only demonstrated per-process Linux resource behavior is implemented here.
WP-A05/ACC-09 remain partial pending broader resource/platform and deployment
evidence. Aggregate process-tree CPU/RSS, GPU limits and escaped-session
containment are not claimed. WP-A04 stays partial, ACC-05 incomplete and Gate A
open: durable startup/board integration, physical safe response and timing,
stack/resource measurements require their own evidence. Host/kernel tests and
AVR compilation are not physical hardware validation.
