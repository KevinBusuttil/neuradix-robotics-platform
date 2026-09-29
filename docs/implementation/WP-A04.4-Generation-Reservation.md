# WP-A04.4 — Durable generation reservation for trusted startup

Status: **integrated.** [PR #24](https://github.com/KevinBusuttil/neuradix-robotics-platform/pull/24) merged as `afe056d30d90254bb741f90b53f3efd42c772564` on 29 September 2026 with a tree
identical to reviewed head `86cd1af`; PR CI 36504330905, push CI 36504325736 and
[post-merge main CI 36523583284](https://github.com/KevinBusuttil/neuradix-robotics-platform/actions/runs/36523583284) passed all four jobs (host, Arduino Uno codec ABI, MCU cross-compilation,
release fault campaign). WP-A04, WP-A08, ACC-05 and Gate A remain
open. This increment makes no hardware support, board storage, physical safety or
Gate A completion claim.

## Verified baseline and scope

Main `0283c93e8634a6f1bf9672528c9b04e1b0071fca` (PR #23, docs only, after PR #22)
was the base; no open pull request owned this work. The increment closes the
long-standing "no allocator is supplied" gap recorded in
[WP-A04.2](WP-A04.2-Command-Freshness.md#trusted-generations-renewal-and-restart),
[WP-A08 host](WP-A08-Actuator-Permission-Binding.md) and
[WP-A08 embedded](WP-A08-Embedded-Actuator-Permission.md) evidence:

- `neuradix_command_core::reservation` — a portable, `no_std`, allocation-free,
  executor/HAL/transport-neutral generation allocator over a two-slot store trait;
- trusted-only provisioning behind a non-default `provisioning` feature;
- strict reserved-permission hooks on **both** actuator boundaries;
- `neuradix_safety::reservation::FileReservationStore`, a Linux host store;
- host fault-injection, subprocess-exit, allocation and MCU cross-compilation evidence.

It adds no board store, provisioning tool, CLI subcommand, epoch registry,
networking, graph executor or Manufacturing logic, and no new dependency.

## Obligation mapping

| WP-A04.2 / plan text | Where it is met | Evidence |
|---|---|---|
| "Before enabling ingress on every startup, trusted initialization must atomically reserve and durably persist a nonzero u128 generation greater than every value previously used" | `GenerationReserver::open` is read-only; the first `reserve` after every `open` always commits the new ceiling to **both** slots with byte-exact read-back before returning a value; values are `(epoch << 64) \| counter`, counter ≥ 1 | R6, R16, E2, E11, H6, H10 |
| "then provision the gates and authorized source with it" | `DriverPermission::reserved` binds the token to the binding key and exact lease generation; `new_reserved` adapters refuse anything else | E1–E7, H11 |
| "The same allocator must cover gate reconstruction, replacement, revocation/regrant and recovery" | One reserver per key for the process lifetime; `reserve_from_window` for zero-I/O replacement; reconstruction takes a fresh token | E8, E10, E15–E18, E22, H13 |
| "including rollback or restored storage" | Required `RollbackDefense` (floor, witness, both, or explicit `Unprotected`); max-selection covers a single-slot rollback **provided no further fault hits the other slot before the next complete two-slot commit** (see proof step 7) | R8, R22, R27, R28, H8 |
| "Reserve before activation so a crash cannot reuse an active value" | Nothing is issued until both slots verify the ceiling; burned values are never reissued | F1, F4, E12, E13, H6 |
| "Storage loss or uncertain allocation requires remaining locally safe until a new, provably unused namespace is established by trusted provisioning" | Blank/corrupt/below-floor/rolled-back/exhausted storage fails `open` with `Remedy::Provision`; the adapter is never granted and ticks write only the safe output. See the amendment below for interrupted commits | R5, R9, E14 |
| "No command can supply that decision" | No port, payload or report can reach a store, reserver, token or provisioning; firmware cannot link `provision` or the record encoders (`record::encode`/`seal` are provisioning-only) | compile-fail doctests; MCU CI gate |
| "Generation u128::MAX cannot be replaced ... wrapping is forbidden" | Counters never wrap (checked arithmetic; release keeps overflow checks); `u128::MAX` is issued at most once; provisioning refuses non-newer epochs | R12 |
| ACC-05 "reboot a device ... old leases cannot be resurrected" | A restarted receiver with a clock restarted at 0 rejects replayed old-generation commands with fresh timestamps | E11, H10 (software level only) |
| ACC-05 timer rollover | A latched clock regression requires reconstruction with a fresh token; old-generation commands are rejected | E22, H13 |
| NRX-PLAT-006 epoch validation | Epoch is the generation's high 64 bits; commands echo the full generation | E11 |
| NRX-EMB-009 bounded allocation | No heap; fixed 64-byte buffers; bounded store calls per operation | allocation tests; MCU allocator scan |
| NRX-EMB-013 executor/transport independence | Synchronous `ReservationStore` trait with no executor, HAL or transport types | MCU cross-compilation |

### Normative amendment: "uncertain allocation" (flagged for explicit review)

This PR amends WP-A04.2. **Uncertain allocation** means no readable record for
the receiver/binding proves a ceiling at or above every issued value: `open`
fails `Blank`, `Corrupt`, `BelowFloor`, `RolledBack` or `Exhausted`
(`Remedy::Provision`). An interrupted or failed commit (`Uncertain`/`Poisoned`),
`StoreUnavailable` or a transient `Unreadable` is **not** uncertain allocation:
nothing in the attempted window was issued, and a read-only reopen with a fresh
store handle selects a ceiling at or above every issued value under C1–C8 (proof
steps 1–4). Deployments may adopt the stricter policy of provisioning a new
namespace after any uncertain commit; no API change is needed. If reviewers
reject this amendment, `remedy()` must map `Uncertain`, `Poisoned`,
`StoreUnavailable` and `Unreadable` to `Provision` for Live.

## API surface (trusted setup, board support and provisioning only)

Nothing new is component-visible: `ActuatorPort::tick` is unchanged and command
metadata still only echoes a `Generation`.

| Item | Purpose |
|---|---|
| `ReservationStore` | Synchronous two-slot store; `read`/`write` of 64-byte records; contract C1–C8 |
| `ReceiverId`, `BindingKey`, `ReservationKey` | Device identity from outside the image (rejects unprogrammed OTP patterns); FNV-1a-64 binding key (mix-up detector, not a credential) |
| `NamespaceEpoch`, `RollbackDefense`, `ReserverConfig` | Registry epoch; required rollback posture (no default); commit window |
| `GenerationReserver::{open, reserve, reserve_from_window, status, into_store}` | Read-only open (2 reads); verified commit (≤ 6 calls); zero-I/O window issue; zero-I/O status |
| `ReservedGeneration` | Not `Clone`/`Copy`/`Default`, private fields, `#[must_use]`; consumed by at most one permission |
| `OpenError`, `ReserveError`, `Remedy`, `OpenFailure<S>` | Typed fail-closed outcomes; refused open returns the store |
| `provision`, `ProvisionGuards` (feature `provisioning`) | Out-of-band epoch installation; never erases; refusals write nothing |
| `record::{decode, crc32}`; `record::{encode, seal}` (feature `provisioning`) | Inspection is always available; record encoders are provisioning/test tooling only, so firmware cannot link a helper that fabricates records |
| `DriverPermission::reserved`, `is_reserved` (both boundaries) | Token-backed permission; `ReservationMismatch` for another binding key or generation |
| `ActuatorAdapter::new_reserved`, `requires_reserved_generations` | Strict adapter; one-way latch after the first reserved grant on a legacy adapter |
| `ActuatorBinding::reservation_key` | `BindingKey::numeric` (embedded) or `BindingKey::named` (host) |
| `FileReservationStore` (Unix) | Linux host store |

Grant check order on both boundaries: shutdown/driver fault, binding, mode,
**reservation requirement**, control time, generation. A failed grant changes no
state (including the latch) and makes no driver call.

## Record format v1 (normative)

64 bytes, little-endian; the same logical record is mirrored in slots A and B,
differing only in the slot tag and CRC.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `"NRXG"` |
| 4 | 1 | format version `0x01` |
| 5 | 1 | slot tag (0 = A, 1 = B) |
| 6 | 2 | flags, 0 (nonzero with a valid CRC is Unsupported) |
| 8 | 16 | ReceiverId |
| 24 | 8 | BindingKey |
| 32 | 8 | namespace epoch, nonzero |
| 40 | 8 | high-water counter: every counter ≤ this may have been issued |
| 48 | 4 | commit count, nonzero, saturating, diagnostic only |
| 52 | 8 | reserved, 0 (nonzero with a valid CRC is Unsupported) |
| 60 | 4 | CRC-32/IEEE over bytes 0..60 (`crc32(b"123456789") = 0xCBF43926`) |

Decode order: uniform 0xFF/0x00 → Blank; magic or CRC mismatch → Corrupt (torn
records land here); CRC-valid unknown version/flags/reserved → Unsupported (fails
closed, even beside a valid slot); CRC-valid bad tag, zero epoch, zero commits or
unprogrammed receiver → Corrupt. **Forward rule:** every future format keeps the
magic, version byte and a v1-style CRC at 60..64 so v1 readers classify it as
Unsupported; there is no in-place migration; downgrade requires trusted erase and
a new epoch.

Golden vectors (receiver bytes `01..10`, binding `numeric(1,2,7,Live)` =
`0x4bf157b8d1af9724`):

- A, epoch 1, high-water 1, commits 1: `4e525847010000000102030405060708090a0b0c0d0e0f102497afd1b857f14b01000000000000000100000000000000010000000000000000000000db2299eb`
- B, same: `4e525847010100000102030405060708090a0b0c0d0e0f102497afd1b857f14b01000000000000000100000000000000010000000000000000000000cb919ac9`
- A, epoch and high-water `u64::MAX`, commits 7: `4e525847010000000102030405060708090a0b0c0d0e0f102497afd1b857f14bffffffffffffffffffffffffffffffff070000000000000000000000712ca9a0`

BindingKey goldens: `numeric(1,2,7,Simulation)` = `0x4bf15ab8d1af9c3d`,
`numeric(1,2,8,Live)` = `0x5f8b4b41d4dfc459`,
`named("controller","thrust","driver/one",Live)` = `0x22494187556301ab`;
length-delimited `derive(b"d",[b"a",b"bc"])` = `0x0b505a6c7bb8e219` ≠
`derive(b"d",[b"ab",b"c"])` = `0x4c3546860759f7d9`.

## Store contract C1–C8 (normative; not detectable by the library)

- **C1** `write` → `Ok` means durable until the next write of that slot.
- **C2** A failed or interrupted write affects only the addressed slot; A and B sit
  in independent erase/program units (never one sector, page, ECC word or file).
- **C3** Reads return medium content, never a volatile write-back or XIP cache.
- **C4** No deferred or background writes after a call returns.
- **C5** One store object per slot pair and per (receiver, binding); store types
  are not `Clone`/`Copy`; no aliasing handle.
- **C6** Erased slots read uniform 0xFF/0x00 or fail the CRC.
- **C7** Synchronous, bounded by the deployment budget, no executor/HAL/transport types.
- **C8** The region is never part of a firmware, factory or update image, backup,
  snapshot or programmer dump that can be written back; a reflash preserves or
  erases it. Writing back an old copy is a rollback.

**Board-store qualification obligation.** Any board store — including
log-structured or wear-levelled ones — must pass this fault suite on its real
driver and later a power-cut rig before it is claimed (B04/B05).

## Algorithm and proof

`open`: read A then B (exactly 2 calls, never a write), classify each slot, and
fail closed in precedence order — handle unavailable, unsupported format, slot
aliasing, foreign receiver, foreign binding, then no record for the key
(`Unreadable` if any Io, `Blank` if both blank, else `Corrupt`) — then apply the
floor, witness and exhaustion checks to the maximum `(epoch, high_water)` Mine
record. One corrupt, blank or unreadable slot is tolerated.

`reserve`: if the next counter exceeds the committed ceiling, commit
`C' = n + window − 1` (clamped, never crossing the epoch): re-read both slots; poison
on an unavailable handle, on content that contradicts this instance (unsupported,
aliased, foreign, a higher Mine record, or no current copy although both decoded:
`MediaChanged`), or on a read error while no slot is confirmed current
(`ReadFailed`); otherwise write first a slot that is **not** a confirmed current
copy (A if both are current), read it back byte-exact, then the other. Any failure
poisons permanently and issues nothing. Only then is `n` returned.

**Proof sketch.** Let M be the highest value ever returned.
1. A returned value is ≤ a ceiling present in both slots when it is returned.
2. Each commit writes first a slot other than one that the pre-commit read
   confirmed holds the current ceiling, and starts the second write only after the
   first slot verified C′ ≥ M. So during every write, an untouched slot holds a
   valid record ≥ M.
3. A write in progress affects only its own slot (C2); a torn slot decodes as
   non-valid, or as its complete old or new content. The new content is ≥ M, and
   the old content is ≥ M **unless that slot was rolled back on its own** (a
   single-slot rollback leaves a valid record below M in it).
4. Open takes the maximum, so after any power losses, a single-slot tear, rot,
   marginal bit, unreadable slot or single-slot rollback, the next value is > M.
5. Provisioning installs only a strictly greater epoch (and, with guards, one above
   prior history).
6. Floor and witness only refuse; they never raise or select a value.
7. Outside the proof (named obligations): faithful two-slot rollback under
   `Unprotected`, or within an epoch under `Floor` only; lying durable-write
   acknowledgements; violations of C2, C3, C5 or C8; and the double fault
   **"single-slot rollback plus any loss, unreadability, rot, erase or
   marginal/torn write of the other slot before the next complete two-slot
   commit"**. After a single-slot rollback the reserver cannot distinguish a
   leftover never-issued ceiling in one slot from a good ceiling beside a
   rolled-back slot, and each case needs the opposite write order, so no slot
   order closes it (a review reproduction: roll back slot A alone, interrupt the
   next commit marginally on A, then erase B during the following commit — the
   next boot reissues). A `Witness` detects it (`RolledBack`); `Floor` detects it
   only across epochs. A single-slot rollback with no further fault is SAFE.

## Normative rules

- **Rollback posture.** `ReserverConfig::new` requires a `RollbackDefense`.
  `Floor` detects cross-epoch restores; `Witness` (the highest generation ever
  provisioned to a source, from a trusted durable record outside the image)
  detects restores within and across epochs; `Unprotected` is explicit, reported
  by `status().rollback`, and leaves the WP-A04.2 rollback obligation **unmet**
  by the library. Reviews must reject `Unprotected` for Live deployments without
  C8 controls. A witness only narrows; an untrusted high witness is safe but a
  denial-of-service vector. The witness is accepted only by `open`, so it can
  never be applied after values were issued.
- **Scheduling.** A store-touching `reserve` may run only before ingress, after
  `revoke` applied the safe output on every adapter served by the executor, or in a
  context that cannot delay evaluation ticks. Replacement while Granted uses
  `reserve_from_window` or a spare (0 store calls); on `CommitRequired`, revoke
  first, then `reserve`, then grant.
- **Spares and activation order.** A spare must have been issued after the most
  recently activated token; drop older spares on every activation, so every
  generation handed to a gate or source was never issued before and exceeds every
  earlier activation, including across adapter reconstruction (E18).
- **Provisioner protocol.** (1) Take the epoch from a durable, fleet-wide, strictly
  increasing, single-use registry; (2) durably advance any floor source first;
  (3) for prior Live history from another allocator pass `prior_high_water` and
  open the first reserved boot with `Witness(legacy_max)`; (4) call `provision`
  over a channel separate from actuator command ingress, only while no grant is
  active, retrying with a NEW epoch until `Ok` before enabling ingress; (5) never
  reuse an epoch and never call `provision` from the boot path. Actuator firmware
  is built without the `provisioning` feature (CI asserts this for the MCU target).
  **No provisioning tool, CLI subcommand or epoch registry is supplied;**
  `reserved_startup` only demonstrates the calls.
- **Startup error routing.** `new_reserved` (no I/O) → `tick(None)` (safe output)
  → `open` (log the posture) → `reserve` → `DriverPermission::reserved` → `grant`
  → optional spare → provision the source → enable ingress. Every error after
  construction routes to one fail-closed path: never grant, keep ticking with
  `None` (safe output only), report `remedy()`, wait for trusted provisioning or
  maintenance.
- **Topology.** Reservation is local to the receiver that owns the gate and its
  store. Storage-less or remote receivers are unsupported; they stay on legacy
  `new()` with the WP-A04.2 obligation documented as theirs.

## Wear and sizing

Each commit writes both slots. A dedicated 10k-cycle internal-flash unit supports
about 10k startups plus window refills; a 1 Hz reset loop exhausts it in about 3 h
(10 Hz: about 17 min), after which the node fails closed permanently. Reserve only
once non-storage startup checks pass and a grant is imminent, optionally rate-limit
the first commit, prefer EEPROM, FRAM, external NOR or a qualified log-structured
slot, size the window from `status().window_remaining`, and monitor
`status().commits`. Each (receiver, binding) needs two independent erase units
(a six-thruster node: 12 internal-flash units, or 128 B per binding on EEPROM/FRAM).
A receiver-wide key is deferred.

## Host store

`FileReservationStore::open` requires a **canonically spelled** path (no trailing
`/`, no `.` or `..` components, no repeated separators, a plain final name; a
trailing `/` or `/.` would otherwise make `lstat` follow a final symlink and `..`
would redirect the parent check) naming an existing directory with no group or
other permission bits, whose parent is not group/world-writable without the sticky
bit. It never creates the directory, refuses symlinks and replaced directories
((dev, ino) re-checked), and takes an exclusive `File::try_lock` on `lock`. The
lock entry is never followed: an existing entry must be a regular file and the
opened file must be that entry; a missing one is created with `O_CREAT|O_EXCL`.
Stale `*.tmp` files are removed. Writes unlink any `slot-x.tmp` entry, create it
exclusively (0600; a planted symlink is never followed), `fsync`, `rename`, and
`fsync` the directory, with directory identity re-verified before every operation
and after the rename. Reads compare the opened slot file with the `lstat`ed entry.
Any write-path error poisons the handle permanently (later reads → `Unavailable` →
`Remedy::ReopenStore`). Reads are bounded to 65 bytes; wrong-length files decode
as Corrupt; missing files as Blank.

Claimed and CI-tested on Linux: per-slot atomic replacement, exclusive locking
in-process and cross-process, refusal of missing/insecure/symlinked/replaced
directories, non-canonical spellings and insecure parents, lock and temporary-file
symlinks not followed, stale temporary files, damaged slot files, process exit at
every store call, injected failure after every internal write step, and directory
replacement between the rename and the identity re-check (S1–S9, H1–H14). Claimed
by design only: durability on ext4/xfs/btrfs with default barriers on devices
honouring flushes. Not claimed: NFS, SMB, FUSE, tmpfs, overlay/ephemeral layers,
lying devices or hypervisors, snapshots or restored copies (use `Witness` or
`Floor`, exclude the directory from snapshots, provision a new epoch after any
restore), writers bypassing the lock, privileged administrators, ancestors above
the parent, directory ownership (running the store as a user other than the
directory owner is an obligation), `FileStoreError::Replaced` from `open` (only a
check-to-open race reaches it; untested), Windows, and the path TOCTOU residual
between an identity check and the next path operation (std has no `openat`).

## Failure matrix (summary)

SAFE = correct given C1–C8; DET = detected, fails closed with a typed remedy;
AVAIL = safe but unavailable; OBL = deployment obligation; CHAR = characterization
test pins the unsafe behaviour.

| Class | Outcome | Tests |
|---|---|---|
| Power loss at any store call, any tear, any pair | SAFE; never forces re-provisioning | F1, F4, E13, H6 |
| Crash after reserve, before grant or source provisioning | SAFE (value burned) | R6, E12 |
| Write error, partial or complete persistence | DET `Uncertain` → `Poisoned`, `ReopenStore` | R17, S1, H7 |
| Silent write drop | DET `VerifyFailed` | R21 |
| Lying cache / flush | OBL (C1, C3) | R20 CHAR |
| One slot unreadable, ECC-flagged, rotted or marginal | SAFE | F2, R15, R19 |
| Single-slot rollback, no further fault | SAFE (maximum selected) | R28 witness case |
| Single-slot rollback plus a later fault on the other slot before the next complete commit | OBL unless a witness is configured (then DET `RolledBack`) | R28, R28b CHAR (unreadable, ECC-flagged, erased, rotted); marginal-slot review reproduction |
| Marginal bits differing within a boot | SAFE for reuse; may fail closed typed | F2b |
| ECC-refuse medium | AVAIL, never reuse | F3, R25 |
| Both slots Io / handle unavailable | DET `Unreadable` / `StoreUnavailable` | R5, H7 |
| Content changed under a live reserver | DET `MediaChanged` | R18 |
| Unsupported format, aliasing, foreign receiver/binding | DET, 0 writes | R3, R10, R11, H9 |
| Clone with the same ReceiverId | OBL | R23 CHAR |
| Two-slot rollback within an epoch | `Witness`: DET `RolledBack`; `Unprotected`/`Floor`: OBL | R8, R22, R27, H8 |
| Rollback across an epoch | DET `BelowFloor` / `RolledBack` | R9, R22, R27 |
| Unreadable slot plus rollback of the other | OBL unless a witness is configured | R28 |
| Counter or `u128::MAX` exhaustion | DET `Exhausted`; never wraps | R12 |
| Token for another binding/mode/generation | DET `ReservationMismatch` | E3–E5, H11 |
| Token from another receiver with the same binding | OBL | E23 CHAR |
| Unreserved permission to strict/latched adapter | DET `ReservationRequired`, 0 driver calls | E1, E6, E19, H11 |
| Legacy grant ≥ 2^64 blocks lower reserved grant | CHAR; migrate with `prior_high_water`/witness | E21, R26 |
| Host directory replaced after open | DET, poisoned, nothing written to the replacement | H14, S5 |
| Boot code auto-provisioning | Firmware cannot link `provision` or `record::encode`/`seal` (CI symbol gate; the resolved-feature gate refuses `provisioning` in the firmware graph); hand-writing records from the documented format and host tooling remain review obligations (OBL) | MCU gates |
| Reset/brown-out loop | AVAIL (wear-out), residual risk | — |

## Mutation checklist

Each mutant, applied to a scratch copy, must fail the named tests:
M1 write the only current slot first (F1, F4, R19); M2 single-slot ping-pong
(R17, F1); M3 min selection (F1, R15); M4 issue despite write/verify error (R17,
R21); M5 `next = high_water` (R6); M6 no read-back (R21); M7 ignore the slot tag
(R11); M8 unknown format treated as Corrupt (R3); M9 skip the floor (R9, R22); M10
no pre-commit read (R18, R19); M11 open repairs by writing (R16); M12 wrapping
arithmetic (R12); M13 skip the witness (R8, R26, R27); M14 remove the
`ReservationRequired` check (E1, E6, H11); M15 drop the key equality (E4, E5,
H11); M16 drop the generation equality (E3, H11); M17 never set the latch (E6);
M18 set the latch before success (E20); M19 `reserve_from_window` commits (R7,
E17); M20 `ReadFailed` collapsed into `MediaChanged` (R29).

**Result:** every checklist mutant M1–M20 was applied to a scratch copy and was
killed by at least one of its named tests, including separate embedded and host
variants of M14–M18 (the host variants are killed by H11) and an extra
provisioning-floor variant (killed by R9, R13). One extra sub-mutant of M12
(`open` computing `next` with a wrapping add) is equivalent: `select` refuses a
`u64::MAX` high-water with `Exhausted` before that addition runs. Review
mutants beyond the checklist were also killed: the post-rename identity check
(S6), the host grant-order reordering (H11), 26 additional allocator/codec
mutants against the R suite, and M1/M3 variants against the F sweeps' proof-step-2
oracle.

## Evidence

Local environment: Rust/Cargo 1.94.1 (pinned), `RUSTFLAGS=-D warnings`, locked
dependencies, AVR GCC from Ubuntu packages, Linux container running as root
(host-store tests also passed as an unprivileged user). Figures are at the
pull request's head `86cd1af`; remote CI results are in the status line above.

| Check | Result |
|---|---|
| fmt; workspace all-target Clippy (warnings denied) | Passed |
| command-core unit U1–U6; deterministic R1–R29 (+R14b, R28b); allocation | 6; 37; 1 passed (0 allocations on every reservation path) |
| Default fault sweeps F1–F5 (debug) | 6 passed; with R and allocation ≈2.3 s wall once built |
| Release sweeps F6–F8 (`--ignored`, separate CI job) | 3 passed in 7.7 s: F6 344,736 single-fault cases (73,728 lose availability under ECC-refuse, never reuse); F7 4,273 double-fault cases; F8 100,000 boots, 49,927 of 49,927 planned faults took effect, 18,459 weak tails left, 598 epochs |
| Embedded E1–E23; reserved allocation scenario | 23; 1 passed |
| Host S1–S9 (store step faults, bounded reads, replacement, canonical paths, symlinks) and 3 lineage unit tests | 13 passed |
| Host H1–H14 (subprocess harness, exit at every store call) | 15 passed (also as an unprivileged user) |
| Doctests incl. compile-fail ownership checks with passing twins | command-core 14, embedded-core 19, safety 12 |
| MCU target T1 and footprint asserts | 3 passed |
| Command/safety/embedded/transport regression group | 259 passed (3 release sweeps ignored) |
| Workspace tests/doctests excluding `neuradix-python` (root container) | 503 passed, 5 ignored (2 AVR, 3 release sweeps); 379 before this increment |
| `neuradix-python` as an unprivileged user | All passed except `privileged_launcher_rejected`, which needs passwordless sudo (CI provides it); SDK 6 passed |
| Examples: reserved_startup, guarded_actuator, embedded-propulsion, minimal-depth-stream, auv-depth-sim; graph/replay examples | Passed |
| no_std checks: time, command-core (with and without `provisioning`), embedded-transport, embedded-core, embedded-actuator-target | Passed |
| MCAP fixtures, import memory and export verification | Passed (`/usr/bin/time` installed locally) |
| Actual ATmega328P codec ABI (AVR job) | 2 passed; does not build this Rust code for AVR |
| `cargo doc --workspace --no-deps` | Passed, 0 warnings |
| `tools/ci/mcu_actuator.sh` (thumbv6m, thumbv7em, riscv32imc, release) | Passed: no `provisioning` in any resolved firmware feature set; no heap-allocator symbols; no `provision` or `record::encode`/`seal` symbols. Negative probes: a forwarded or command-line `provisioning` feature trips the feature gate, and a feature-enabled build shows the provisioning and encoder symbols the symbol gate rejects |

Footprint on thumbv6m, thumbv7em and riscv32imc (compile-time asserted bounds in
parentheses): adapter 464 B (≤ 640, unchanged by the added `bool`), report 232 B
(≤ 320), `GenerationReserver` with a zero-sized store 72 B (≤ 128),
`ReservedGeneration` 48 B (≤ 64; it carries the epoch as well as the generation
and key). Monomorphized code (object symbol sizes): `reserve_and_install`
1.26–1.53 KiB, `open_reserver` 588–756 B, `replace_from_window` 222–274 B.
Static frame adjustments (`sub sp`/`push`, not measured stack use): the example's
reserved startup chain `reserve_and_install` → `grant_reserved` → `grant` is about
1.87 KiB (thumbv6m), 1.78 KiB (thumbv7em) and 1.79 KiB (riscv32imc), versus about
0.7–0.8 KiB for the legacy `install` chain; the reserver itself fits in the
~0.4 KiB `open_reserver`/`reserve_and_install` frames. Board packages must size
startup stacks from their own measurements. No physical stack, timing or commit
latency is measured.

Corrected during review (before the pull request): the record encoders are now
provisioning-only; the MCU feature gate checks resolved feature sets (the earlier
edge-display gate missed forwarded features) and both gates capture producer
output first so a failing `cargo tree`/`llvm-nm` cannot pass; the host store
refuses non-canonical paths and never follows planted `lock`/temporary-file
symlinks; the F5/F8 campaign's faults now always apply to the op kind they hit
(previously about a third were silent no-ops counted as fired); H11 now pins the
host grant check order; the single-slot-rollback residual was widened (R28b).
After the pull request opened, Codex review found that provisioning over a lone
foreign record beside an empty slot wrote the foreign slot first, so a torn first
write could leave no valid record (still failing closed); provisioning now writes
first to a slot holding nothing worth keeping (R14b).
H14 observes `Uncertain(ReadFailed(A))` for a directory replaced before a commit
(the pre-commit read sees the replacement first) and `WriteFailed(A)` when it is
replaced between that read and the write; both poison the handle and write
nothing to the replacement. F7's exact count (4,273) is below the design's rough
estimate because the design's stride and timeline give about 5×10^3 cases.

### Not established

- **No hardware execution** and no board `ReservationStore`: nothing here
  exercises real flash, EEPROM, FRAM, ECC behaviour, power cuts, reset paths,
  commit latency or stack margins on an MCU.
- **Host durability under real power loss** is argued by design only (no
  power-cut or dm-flakey rig).
- **Rust AVR** is not built (nightly-only); the AVR job checks the generated
  C/C++ codec ABI only.
- The fault model abstracts vendor flash physics; board qualification must re-run
  the suite against the real driver.

## Constraints, deferrals and non-claims

- The embedded-core and safety test targets include
  `crates/command-core/tests/support/fault_store.rs` through `#[path]`; `cargo
  package` would omit it, so those test targets do not build from a packaged crate.
  The crates are unpublished (0.0.1); a dev-only support crate is deferred.
- Deferred: a monotonic-anchor record extension, board `ReservationStore`
  implementations and qualification (B04/B05), breaking enforcement of reserved
  permissions for `ExecutionMode::Live`, host `SafetyGate`/`LeaseTable`
  multi-binding wiring, a receiver-wide key, remote/gateway reservation, a
  provisioning CLI and registry.
- Not claimed: hardware support, board storage or execution; physical safe
  response; stack, timing or commit-latency figures; durability of
  `VolatileTestSlots` or any store violating C1–C8; rollback detection within an
  epoch under `Unprotected` or `Floor` alone; lying acknowledgements; same-id
  clones; forged records (no MAC); authentication of commands or of the
  provisioning channel; secure boot or dual-bank rollback (Embedded Plan v0.2 §6) —
  the two slots are not firmware banks and wired reflash is covered by C8.
- Enforcement is opt-in (`new_reserved`, or the latch after the first reserved
  grant); legacy `new` adapters and direct `SafetyGate`/`LeaseTable`/`CommandGate`/
  `PropulsionNode` users are not enforced.
- This closes only "no allocator is supplied": WP-A08 remaining item 2 becomes
  partial (a board store remains), and ACC-05 reboot/epoch evidence exists at
  software level only.

## Remaining Gate A acceptance and next task

Gate A remains open. After this increment:

- **A01/ACC-01** — broader evidence inventory and optional-tool audit.
- **A02/ACC-03** — transport binding, compact-ID collision handling and a
  recording migration fixture.
- **A03/ACC-02** — board-generated payloads matching host golden vectors on the
  actual Uno; stack/timing.
- **A04/ACC-05** — a qualified board `ReservationStore`, trusted startup on the
  rig, reboot/timer-rollover evidence on hardware and the rig's documented
  physical safe response.
- **A05/ACC-09** — aggregate process-tree resources and additional OS/deployment
  containment.
- **A06/ACC-08** — broader interchange and scale qualification.
- **A07/ACC-07** — closed-loop replay evidence.
- **A08** — graph-to-binding through trusted setup (with B02/B07), the board
  store, board driver qualification, and a follow-up making reserved permissions
  mandatory for `ExecutionMode::Live`.

**Next recommended task (Gate A, software-only):** WP-A02's transport binding
and compact-ID collision handling with a recording migration fixture (ACC-03);
it needs no hardware and is the remaining Gate A item the later board packages
depend on. Board-store qualification follows once a board package exists
(B04/B05). This does not pull Gate B or Manufacturing work forward.
