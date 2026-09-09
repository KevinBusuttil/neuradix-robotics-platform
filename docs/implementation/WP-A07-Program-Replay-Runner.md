# WP-A07 — Reusable single-processor replay runner

Status: [PR #16](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/16) merged as `88c8eb1015a96d5314b1314b623758c7d43100f1` after current-head review and passing PR CI 34380936422/push CI 34380929961. WP-A07/ACC-07 remain partial and Gate A stays open.

## Baseline and review

GitHub main was verified as `3e47e7f973445d381473b3d35c1c9ee14452187a`.
[PR #15](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/15)
was already merged with unchanged head `329866aceec9db48d416e858ad8c479036ab08a7`;
[main CI 34350022456](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34350022456)
passed. No review comments or required reviews were present; main was unprotected
and rulesets empty. Review covered streaming admission/finalization, latched errors,
legacy conversion, historical reads and independent export/memory evidence. No
concrete prerequisite defect required a code change. The full A06 suites are rerun.
PR #13/#14 integration is not repeated; A05 implementation remains unchanged.

An isolated checkout preserves existing work and branches. WP-A01's integrated
source/compiler baseline supplies the prerequisite; its broader evidence inventory
and optional-tool audit remain partial.

## Problem and design

`run_lockstep` and `lockstep_replay.rs` demonstrated a recording-backed processor
test, but callers had to assemble fresh state, schedule, expected outputs and
comparison themselves. CLI `replay run` only hashes recorded data; it cannot prove
a selected changed program ran. The new additive `neuradix-runtime::replay` API
provides a reusable bounded single-processor execution/comparison facility.
There are no new dependencies or lockfile changes and no recording dependency is
added to runtime production code. Architecture directions remain unchanged.

`ReplayCase::new` copies a declared program/component label, exact configuration
bytes, seed and starting evaluation timestamp. Checked `push` copies each input
and its ordered expected output batch only after time/count/storage admission.
Fields carrying invariants are private; immutable getters expose audit data.
Rejected insertions return typed errors and leave the prior admitted case unchanged;
callers must handle them rather than silently treating a prefix as the intended case.
Running borrows the completed case, preventing concurrent mutation.

`ReplayProgram` selects an actual compiled Processor type and supplies a factory.
Each run checks the case/factory labels, calls the factory once, then invokes the
returned processor in insertion order. Factory construction receives the immutable
configuration and seed. A new private ManualClock begins at the case start.
Configuration decoding and input representation belong to the selected program;
invalid representations must return ComponentError, as the example demonstrates.

The label match is verified equality of caller declarations, not a verified binary
digest. The report's Rust type name identifies the selected compiled type for
diagnostics; it is not stable artifact identity or authentication. Actual call
counters and changed-code regression results demonstrate invocation in the tested
process. The factory must create isolated initial state and honor config/seed;
this trait cannot prevent shared mutable state, ambient clocks or I/O. A seed is
passed explicitly but the runner does not claim to verify arbitrary RNG usage.

## Clock and comparison policy

Evaluation timestamps are an explicit trusted replay schedule, separate from
`ReplayInput.source_time`. Missing source time remains unknown; another source
domain is retained without comparison or clock conversion. Source values never
set TickContext.now. All evaluation times must match the starting domain, be
nondecreasing and have representable adjacent i128 elapsed nanoseconds. Equality
is accepted and preserves insertion order; negative epochs are valid. Clock
mismatch, regression and subtraction overflow reject before copying/admission.
TickContext.sequence is the zero-based admitted index. No wall clock or sleep
drives replay time. Gaps do not synthesize idle ticks: callers must explicitly
supply every tick/input required by the program, including expiry/watchdog ticks.

Expected outputs are grouped by evaluation tick, and compared byte-for-byte in
returned order. A reordered batch yields Changed slots; output at the wrong tick
yields missing/extra/changed positions. There is no tolerance, sorting, matching
by identity or resynchronization. Comparison includes timestamp bytes only when
the selected output encoding contains them; evaluation-tick attribution is always
part of the comparison. Payload semantics/ROS decoding are not inferred.

Matched means every admitted step completed and all batches agreed. An empty case
initializes but reports zero invocations; its vacuous match is not execution proof.
Mismatched means complete execution with differences. IdentityMismatch,
InitializationFailed, ProcessorFailed and OutputLimit are explicit unsuccessful
outcomes; failures stop the run and cannot become successful comparison.
The report records calls entered (including the failing call), completed admitted
batches, admitted output count, total positional differences and a diagnostic
prefix. A fixed 256-byte UTF-8 error prefix preserves the ComponentError category
and a truncation flag. No unbounded error text or output history is retained.

## Bounds and ownership

`ReplayLimits::new` validates inclusive limits before it can be used:

| Limit | Default | Allowed range |
|---|---:|---:|
| Input/evaluation count | 10,000 | 1–1,000,000 |
| Expected and actual output count, independently | 100,000 | 1–1,000,000 |
| Configuration/input/output payload bytes | 1 MiB | 1 B–16 MiB |
| Accounted retained case storage | 64 MiB | 1 KiB–256 MiB |
| Retained mismatch slots | 64 | 0–4,096 |
| Diagnostic array bytes | 16 KiB | 0–1 MiB |

The declared identity is 1–256 UTF-8 bytes. Case accounting is 512 bytes plus label
and configuration bytes, then 256 bytes plus input bytes per step, and 64 bytes
plus payload bytes per expected output. The allowances cover collection headers
and step Vec growth for this host representation; they are not an exact allocator
or process RSS promise. Checked additions precede copying. Output/difference
counters are bounded by the validated counts (differences <= expected + actual).
Diagnostic count × size_of(ReplayMismatch) must fit the report byte budget; the
array is allocated once and stops growing at that count. Remaining differences
are counted by `omitted`. Fixed report fields, including its fixed failure prefix,
are outside that array budget. The report borrows rather than copies the case.

One bounded input copy is handed to each process call. Processor's existing API
returns a Vec of output Vecs, so it can allocate an excessive batch before returning.
The runner checks batch count before scanning payload sizes, then compares without
retaining payloads across ticks. Arbitrary factory/processor/Drop allocation, blocking,
panic and side effects remain outside the runner's bounds. External supervision
is required for wall-time/process resource containment. No worker thread, queue,
transport or new sandbox is introduced. Caller-owned source data and post-return
report edits/copies are outside runner-owned storage.

## Migration and example

`Processor`, `run_lockstep`, record formats and CLI digest behavior are unchanged.
New code implements `ReplayProgram` and wraps its selected Processor with the
opaque replay input/output byte interface. Adapters must validate decoding and
deliberately map recording data to the evaluation schedule; source/log timestamps
alone are not authority to set live control time. Use the bounded recording APIs
when materializing recordings. Live control state is never passed into this runner.

`cargo run --locked -p neuradix-runtime --example program_replay` executes two
compiled accumulator variants against the same case: the unchanged implementation
matches and changed controller logic fails comparison. The input source Sensor
timestamp is independent of Replay evaluation time. Checked decoding/arithmetic
fail explicitly. This is open-loop program execution, not physical outcome evidence.

## Verification

Verified implementation revision `bb20bb45ed9d0960bc0327774183163d66598301`:
[PR CI 34380370682](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34380370682)
and [push CI 34380365669](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/34380365669)
passed with pinned Rust/Cargo 1.94.1 and locked dependencies. Evidence-only follow-up
commits are checked again before review readiness; their runs are linked in PR #16.

- Formatting, all-target Clippy with warnings denied and documentation: passed.
- Runtime: 20 tests/doctests, including seven externally supervised replay scenarios
  and a recording-backed reusable-runner test; executable replay example passed.
- Workspace: 346 tests/doctests passed. Two AVR tests ignored by the generic host
  invocation passed separately in the AVR job (ATmega328P ABI/scalar compilation).
- Preserved focused regressions: A04 88, A05 75, recording 44; Python SDK 6 passed.
- Independent MCAP fixtures/export comparison, integrated examples and four independent
  no_std configurations passed. Import RSS for 8/128 MiB streaming payload was
  4,144/4,216 KiB (64 MiB budget); 128 MiB materialization was 135,044 KiB
  (192 MiB budget); bounded rejection 11,960 KiB (64 MiB budget). Writer RSS for
  8/128 MiB output was 2,996/3,024 KiB (64 MiB budget). These preserve A06 evidence;
  they do not measure arbitrary replay processor memory.
- Local fixture regeneration/check and six SDK tests passed. Changed Markdown:
  8 files, 332 local links/anchors checked with zero errors before this evidence
  update. Full-document scan: 32 pre-existing missing archived spec/image targets
  among 51 files/655 links; failed, outside this increment. Whitespace check passed.

Earlier CI attempts failed on formatting and one unused import; those defects were
corrected before the successful revision above. No failing current compiler gate
is waived. Test totals exclude duplicate child-process result lines.

The seven adversarial replay scenarios use 10-second external child deadlines;
the runtime suite has a 60-second CI wrapper and the example a 30-second wrapper.

Tests cover actual factory/process invocation, fresh state, changed code/config/seed,
source/evaluation separation, order/equal timestamps, clock extremes and invalid
schedules, exact case/count/payload/report boundaries, bounded mismatch/error
retention, missing/extra/reordered outputs, initialization/processor failures and
continued independent conventional processing. Existing CLI digest tests remain
the compatibility evidence; no CLI semantic migration occurs.

Local Rust/rustup/Cargo and AVR tools are unavailable; pinned GitHub CI supplies
compiler execution. No hardware/rig/timing validation is available or claimed.
The existing full-document archive failures remain outside this increment.

## Remaining acceptance

WP-A07 remains partial for broader replay; ACC-07 remains partial while required
closed-loop scenario/tolerance/repeat evidence is missing. General deployment
graphs, counterfactual evaluation, simulation backends and CLI replacement are
deferred. WP-A06/ACC-08 remain partial for broader interchange/scale qualification;
WP-A05/ACC-09 for broader resource/platform work; WP-A04 remains partial, ACC-05
incomplete and Gate A open. Host/no_std/AVR tests do not establish physical safety.
