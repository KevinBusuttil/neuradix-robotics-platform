#!/usr/bin/env bash
# Regenerate, or with --check verify, the legacy scalar fixture using the
# historical producer at the pinned revision. Needs git history and cargo.
set -euo pipefail
REV=c8aa4671bcee8739354beb7880551db7f64314fa
here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
work=$(mktemp -d)
cleanup() {
  git -C "$repo" worktree remove --force "$work/src" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
git -C "$repo" worktree add -q --detach "$work/src" "$REV"
cd "$work/src"
[[ $(git rev-parse HEAD) == "$REV" ]]
cargo run --locked -q -p neuradix-cli -- contract generate "$here/contract.yaml" \
  --language nostd-rust --out-dir "$work/gen" >/dev/null
cargo run --locked -q -p neuradix-cli -- contract generate "$here/depth-reversed.yaml" \
  --language nostd-rust --out-dir "$work/gen" >/dev/null
mkdir -p crates/record/examples
cp "$work/gen/legacy_probe.rs" crates/record/examples/legacy_probe_generated.rs
cp "$work/gen/vehicle_depth.rs" crates/record/examples/vehicle_depth_generated.rs
cp "$here/producer.rs" crates/record/examples/legacy_fixture.rs
cargo run --locked -q -p neuradix-record --example legacy_fixture -- "$here/contract.yaml" "$here/depth-reversed.yaml" \
  > "$work/recording.nrec.hex"
if [[ ${1:-} == --check ]]; then
  diff -u "$here/legacy_probe.generated.rs" "$work/gen/legacy_probe.rs"
  diff -u "$here/vehicle_depth_reversed.generated.rs" "$work/gen/vehicle_depth.rs"
  diff -u "$here/recording.nrec.hex" "$work/recording.nrec.hex"
  echo "legacy fixture matches $REV"
else
  cp "$work/gen/legacy_probe.rs" "$here/legacy_probe.generated.rs"
  cp "$work/gen/vehicle_depth.rs" "$here/vehicle_depth_reversed.generated.rs"
  cp "$work/recording.nrec.hex" "$here/recording.nrec.hex"
  echo "legacy fixture regenerated from $REV"
fi
