#!/usr/bin/env bash
# WP-A08 embedded actuator MCU cross-compilation evidence.
#
# Builds the no_std embedded core and a monomorphized actuator adapter (plus the
# WP-A04.4 reserved startup path) for bare-metal targets, reports the 32-bit type
# footprint, fails if any object references a heap allocator, and fails if the
# trusted `provisioning` feature or its code reaches the firmware graph. This is
# compilation evidence only; it does not execute the adapter on a board. AVR Rust
# is not covered (nightly-only target).
set -euo pipefail
cd "$(dirname "$0")/../.."
target_dir=${CARGO_TARGET_DIR:-target}
targets=(thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imc-unknown-none-elf)
# Prefer the pinned toolchain's llvm-tools component, then a system llvm-nm.
nm="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-nm"
[[ -x "$nm" ]] || nm=$(command -v llvm-nm || true)
for target in "${targets[@]}"; do
  echo "== $target"
  # Provisioning-absence gate, resolved features: in the firmware (normal-edge)
  # graph, no neuradix crate may resolve with `provisioning`, whether requested by
  # a dependency declaration, a default or forwarding feature, or the command
  # line. `{f}` prints each package's RESOLVED feature set (the edge-feature
  # display does not show root or forwarded features). Captured first so a failing
  # `cargo tree` aborts (set -e) instead of passing.
  graph=$(cargo tree --locked -e normal --target "$target" -f '{p} [{f}]' \
    -p neuradix-embedded-core -p neuradix-example-embedded-actuator-target)
  grep -qE 'neuradix-command-core v[^[]*\[' <<<"$graph" \
    || { echo "$target firmware graph lacks neuradix-command-core; gate is blind" >&2; exit 1; }
  if grep -E 'neuradix-[a-z-]+ v[^[]*\[[^]]*\bprovisioning\b' <<<"$graph"; then
    echo "provisioning feature enabled in the $target firmware graph" >&2; exit 1
  fi
  echo "no provisioning feature in the firmware graph"
  # time and command-core are built as no-default-features dependencies.
  # The WP-A02 transport (frames, commands, compact channel envelopes) is built
  # for the same targets; it has no `alloc` crate, so it cannot allocate.
  cargo build --locked --release --target "$target" \
    -p neuradix-embedded-core -p neuradix-embedded-transport \
    -p neuradix-example-embedded-actuator-target
  if [[ -z "$nm" ]]; then
    echo "llvm-nm unavailable: allocator-symbol scan skipped" >&2
    exit 1
  fi
  dir=$(mktemp -d)
  for lib in neuradix_time neuradix_command_core neuradix_embedded_core \
             neuradix_embedded_transport neuradix_example_embedded_actuator_target; do
    mkdir -p "$dir/$lib"
    rlib=$(ls -t "$target_dir/$target/release/deps/lib$lib"-*.rlib | head -n 1)
    rlib=$(readlink -f "$rlib")
    (cd "$dir/$lib" && ar x "$rlib")
  done
  if "$nm" -u "$dir"/*/*.o 2>/dev/null | grep -Ei '__rust_(alloc|dealloc|realloc)|__rdl_|__rg_'; then
    echo "heap allocator reference found for $target" >&2
    exit 1
  fi
  echo "no heap allocator references"
  # Provisioning-absence gate, symbols: no provisioning code in any object.
  # llvm-nm -C leaves legacy-mangled impl paths escaped ("..", not "::"), e.g.
  # `_$LT$neuradix_command_core..reservation..provisioning..ProvisionError...`,
  # which is the symbol a feature-enabled build always emits; match both forms.
  # Symbols are captured first so an llvm-nm failure aborts (set -e) instead of
  # passing, and the always-present reservation symbols prove the pattern's
  # prefix still matches this toolchain's demangled output.
  syms=$("$nm" -C "$dir"/*/*.o)
  reservation='neuradix_command_core(::|\.\.)reservation(::|\.\.)'
  grep -qE "$reservation" <<<"$syms" \
    || { echo "no reservation symbols for $target; provisioning gate is blind" >&2; exit 1; }
  # Also the record encoders, which are provisioning-only: firmware must not link
  # a helper that fabricates records outside the reserver.
  if grep -E "${reservation}(provision|record(::|\.\.)(encode|seal)\b)" <<<"$syms"; then
    echo "provisioning code linked for $target" >&2; exit 1
  fi
  echo "no provisioning symbols"
  size_of=$("$nm" -S "$dir"/neuradix_example_embedded_actuator_target/*.o | grep -c FOOTPRINT || true)
  [[ "$size_of" -ge 1 ]] || { echo "footprint symbol missing" >&2; exit 1; }
  "$nm" -S --size-sort -C "$dir"/neuradix_example_embedded_actuator_target/*.o \
    | grep -E 'embedded_actuator_target::(setup|install|control_step|revoke|shutdown|setup_reserved|open_reserver|reserve_and_install|replace_from_window)|GenerationReserver.*(open|reserve)' || true
  rm -rf "$dir"
done
echo "MCU actuator compilation evidence complete (no hardware execution)."
