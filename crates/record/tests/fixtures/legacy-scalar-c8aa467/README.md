# Legacy scalar recording fixture (c8aa467)

A native Neuradix recording whose scalar payloads were written by the
**historical** embedded code generator, and the explicit provenance needed to
migrate them to `neuradix.scalar-le.v2`. Used by
`crates/record/tests/legacy_migration.rs`.

## Producer

| Item | Value |
| --- | --- |
| Revision | `c8aa4671bcee8739354beb7880551db7f64314fa` (2026-07-20, "Add embedded contract codegen") |
| Generator | `neuradix contract generate --language nostd-rust` at that revision |
| Container writer | `neuradix-record` `NativeRecordWriter` at that revision (`FORMAT_VERSION` 1) |
| Toolchain | Rust 1.94.1 (the revision's pinned `rust-toolchain.toml`) |

The historical wire layout is each scalar field in **authored declaration
order**, fixed-width little-endian, `bool` as one `0`/`1` byte, with no header.
The generated encoders are checked in as evidence:

- `legacy_probe.generated.rs`: `yaw`@0, `pitch`@8, `count`@16, `armed`@20,
  `rate`@21, `ticks`@25, `offset`@33, `flags`@41 (45 bytes). Canonical v2 order
  is alphabetical, so every field moves.
- `vehicle_depth_reversed.generated.rs`: `uncertainty`@0, `depth`@8 (16 bytes).
  Its schema identity equals the standard `vehicle-depth` contract's
  (`sha256:4c9c…2a73`), and its length equals the v2 length. Reading these
  bytes as v2 therefore decodes silently with the two values swapped.

## Files

| File | Content |
| --- | --- |
| `contract.yaml` | Mixed-width probe contract (authored order = legacy order) |
| `depth-reversed.yaml` | `vehicle-depth` with its fields authored in reverse |
| `producer.rs` | Producer program; runs only inside a worktree at the revision |
| `generate.sh` | Regenerates, or with `--check` verifies, the generated sources and recording |
| `recording.nrec.hex` | The recording (hex, 64 columns) |
| `provenance.json` | Migration request (`neuradix.legacy-scalar-provenance.v1`) for channels 1 and 3 |
| `expected.json` | Independently written expected values, timestamps, sequences, opaque bytes, pinned wire IDs and replay digests |

Recording contents:

| Channel | Name | Clock domain | Records | Payload |
| --- | --- | --- | --- | --- |
| 1 | `fixtures/legacy-probe` | monotonic | 3 (sequences 10, 12, 14) | legacy scalar, 45 bytes |
| 2 | `fixtures/opaque-log` | simulation | 2 (sequences 5, 9; one at −10 ns) | opaque bytes, never interpreted |
| 3 | `navigation/vehicle-depth` | monotonic | 2 (sequences 0, 1) | legacy scalar, 16 bytes |

The expected values include `-0.0`, `i32::MIN`, `i64::MIN`, `u64::MAX` and
`u32::MAX`. Every float is exactly representable.

## Regenerating

From a full clone (the revision must be reachable):

```sh
crates/record/tests/fixtures/legacy-scalar-c8aa467/generate.sh --check
```

The script creates a temporary detached worktree at the pinned revision. It then
generates both structs with that revision's CLI, builds `producer.rs` as an
example of that revision's `neuradix-record`, and compares the output with the
checked-in files. Without `--check` it rewrites them. `expected.json` is written
by hand from the values in `producer.rs`, not from the migration code.
