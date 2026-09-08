//! Explicit AVR toolchain gate. Run with:
//! cargo test -p neuradix-embedded-codegen --test avr -- --ignored --nocapture
//! CI runs this gate after installing gcc-avr and avr-libc. These tests compile
//! and link for ATmega328P; they do not claim execution on physical hardware.

mod common;

use std::process::Command;

use neuradix_embedded_codegen::{
    CppTarget, cpp_conformance_main, generate_cpp, generate_cpp_for_target, golden_vectors,
};

use common::{TempDir, depth, require_compiler, success, tiny};

const AVR_FLAGS: &[&str] = &[
    "-mmcu=atmega328p",
    "-std=gnu++11",
    "-Os",
    "-Wall",
    "-Wextra",
    "-Werror",
    "-ffunction-sections",
    "-fdata-sections",
    "-fno-exceptions",
    "-fno-rtti",
];

#[test]
#[ignore = "requires gcc-avr and avr-libc; mandatory in the AVR CI job"]
fn avr_compiles_and_links_supported_scalars_within_uno_memory() {
    let compiler = require_compiler(&["avr-g++"]);
    let dir = TempDir::new("avr-supported");
    let contract = tiny();
    let header = generate_cpp_for_target(&contract, CppTarget::AvrUno).unwrap();
    let harness = cpp_conformance_main(&contract, &golden_vectors(&contract).unwrap()).unwrap();
    std::fs::write(dir.0.join("tiny_telemetry.h"), header.code).unwrap();
    let source = dir.0.join("main.cpp");
    let elf = dir.0.join("conformance.elf");
    std::fs::write(&source, harness).unwrap();
    success(
        Command::new(compiler)
            .args(AVR_FLAGS)
            .arg(source)
            .arg("-Wl,--gc-sections")
            .arg("-o")
            .arg(&elf)
            .output()
            .unwrap(),
    );
    let size = success(Command::new("avr-size").arg(&elf).output().unwrap());
    let output = String::from_utf8(size.stdout).unwrap();
    println!("AVR conformance harness (compile/link only):\n{output}");
    let sizes: Vec<usize> = output
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .take(3)
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(sizes.len(), 3);
    assert!(
        sizes[0] + sizes[1] <= 32_256,
        "exceeds Uno application flash"
    );
    assert!(sizes[1] + sizes[2] <= 2_048, "exceeds Uno static SRAM");
    // Static sections exclude the stack; board runtime headroom remains a HIL gate.
}

#[test]
#[ignore = "requires gcc-avr and avr-libc; mandatory in the AVR CI job"]
fn avr_rejects_portable_binary64_header_with_explicit_diagnostic() {
    let compiler = require_compiler(&["avr-g++"]);
    let dir = TempDir::new("avr-binary64");
    std::fs::write(
        dir.0.join("vehicle_depth.h"),
        generate_cpp(&depth()).unwrap().code,
    )
    .unwrap();
    let source = dir.0.join("main.cpp");
    std::fs::write(
        &source,
        "#include \"vehicle_depth.h\"\nint main() { return 0; }\n",
    )
    .unwrap();
    let output = Command::new(compiler)
        .args(AVR_FLAGS)
        .arg("-c")
        .arg(source)
        .arg("-o")
        .arg(dir.0.join("main.o"))
        .output()
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "AVR must not compile binary64 memcpy code"
    );
    assert!(
        diagnostic.contains("Neuradix float64 requires IEEE-754 binary64"),
        "wrong failure; expected the target ABI guard:\n{diagnostic}"
    );
    println!("Expected AVR binary64 rejection:\n{diagnostic}");
}
