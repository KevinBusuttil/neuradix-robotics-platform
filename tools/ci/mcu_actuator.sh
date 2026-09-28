#!/usr/bin/env bash
# WP-A08 embedded actuator MCU cross-compilation evidence.
#
# Builds the no_std embedded core and a monomorphized actuator adapter for
# bare-metal targets, reports the 32-bit type footprint and fails if any object
# references a heap allocator. This is compilation evidence only; it does not
# execute the adapter on a board. AVR Rust is not covered (nightly-only target).
set -euo pipefail
cd "$(dirname "$0")/../.."
targets=(thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imc-unknown-none-elf)
# Prefer the pinned toolchain's llvm-tools component, then a system llvm-nm.
nm="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-nm"
[[ -x "$nm" ]] || nm=$(command -v llvm-nm || true)
for target in "${targets[@]}"; do
  echo "== $target"
  # time and command-core are built as no-default-features dependencies.
  cargo build --locked --release --target "$target" \
    -p neuradix-embedded-core -p neuradix-example-embedded-actuator-target
  if [[ -z "$nm" ]]; then
    echo "llvm-nm unavailable: allocator-symbol scan skipped" >&2
    exit 1
  fi
  dir=$(mktemp -d)
  for lib in neuradix_time neuradix_command_core neuradix_embedded_core \
             neuradix_example_embedded_actuator_target; do
    mkdir -p "$dir/$lib"
    rlib=$(ls -t "target/$target/release/deps/lib$lib"-*.rlib | head -n 1)
    (cd "$dir/$lib" && ar x "$OLDPWD/$rlib")
  done
  if "$nm" -u "$dir"/*/*.o 2>/dev/null | grep -Ei '__rust_(alloc|dealloc|realloc)|__rdl_|__rg_'; then
    echo "heap allocator reference found for $target" >&2
    exit 1
  fi
  echo "no heap allocator references"
  size_of=$("$nm" -S "$dir"/neuradix_example_embedded_actuator_target/*.o | grep -c FOOTPRINT || true)
  [[ "$size_of" -ge 1 ]] || { echo "footprint symbol missing" >&2; exit 1; }
  "$nm" -S --size-sort -C "$dir"/neuradix_example_embedded_actuator_target/*.o \
    | grep -E 'embedded_actuator_target::(setup|install|control_step|revoke|shutdown)' || true
  rm -rf "$dir"
done
echo "MCU actuator compilation evidence complete (no hardware execution)."

