//! Cross-language conformance: the C++ projection compiled and run against the
//! golden vectors, plus reference-codec and determinism checks.

use std::path::PathBuf;
use std::process::Command;

use neuradix_contracts::{Contract, load_file};
use neuradix_embedded_codegen::{
    ScalarValue, cpp_conformance_main, generate_cpp, generate_nostd_rust, golden_vectors,
};

fn contract() -> Contract {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/standard/navigation/vehicle-depth.yaml");
    load_file(&path).expect("load vehicle-depth contract")
}

#[test]
fn golden_vectors_are_deterministic() {
    let c = contract();
    let a = golden_vectors(&c).unwrap();
    let b = golden_vectors(&c).unwrap();
    assert_eq!(a.wire_len, 16); // two f64 fields
    assert_eq!(a.vectors.len(), 4);
    // Determinism: same bytes both times.
    for (x, y) in a.vectors.iter().zip(&b.vectors) {
        assert_eq!(x.bytes_hex, y.bytes_hex);
        assert_eq!(x.bytes_hex.len(), a.wire_len * 2);
    }
}

#[test]
fn nostd_rust_and_cpp_generate() {
    let c = contract();
    let rust = generate_nostd_rust(&c).unwrap();
    assert_eq!(rust.type_name, "VehicleDepth");
    assert!(rust.code.contains("pub const WIRE_LEN: usize = 16;"));
    assert!(rust.code.contains("fn encode"));
    assert!(rust.code.contains("fn decode"));

    let cpp = generate_cpp(&c).unwrap();
    assert_eq!(cpp.type_name, "VehicleDepth");
    assert!(cpp.code.contains("static constexpr size_t WIRE_LEN = 16;"));
    assert!(cpp.code.contains("size_t encode"));
    assert!(cpp.code.contains("static bool decode"));
}

/// Compile the generated C++ header + a self-checking harness with `g++` and run
/// it: it verifies every golden vector encodes to the expected bytes and round
/// trips. This proves the C++ projection agrees with the Rust reference on the
/// wire. Skips cleanly when no C++ compiler is available.
#[test]
fn cpp_projection_agrees_with_golden_vectors() {
    let compiler = match ["g++", "c++", "clang++"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
    {
        Some(cc) => cc,
        None => {
            eprintln!("no C++ compiler; skipping C++ conformance");
            return;
        }
    };

    let c = contract();
    let cpp = generate_cpp(&c).unwrap();
    let golden = golden_vectors(&c).unwrap();
    let harness = cpp_conformance_main(&c, &golden).unwrap();

    let dir = std::env::temp_dir().join("neuradix-cpp-conformance");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{}.h", cpp.header_name)), &cpp.code).unwrap();
    let main_cpp = dir.join("main.cpp");
    std::fs::write(&main_cpp, &harness).unwrap();
    let bin = dir.join("harness");

    let compile = Command::new(compiler)
        .args(["-std=c++17", "-O2", "-Wall", "-Wextra", "-Werror"])
        .arg(&main_cpp)
        .arg("-I")
        .arg(&dir)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("run compiler");
    assert!(
        compile.status.success(),
        "generated C++ must compile cleanly:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let run = Command::new(&bin).output().expect("run harness");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success() && stdout.contains("ALL_PASS"),
        "C++ harness must pass every golden vector; got status {:?}, stdout:\n{stdout}",
        run.status.code()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The reference encoder (which produces the golden bytes) must itself match the
/// declared vectors — a self-consistency guard.
#[test]
fn reference_encoder_matches_the_golden_bytes() {
    let c = contract();
    let golden = golden_vectors(&c).unwrap();
    let rows = neuradix_embedded_codegen::GoldenSet::value_rows(&c).unwrap();

    for ((_, values), vector) in rows.iter().zip(&golden.vectors) {
        let mut bytes = Vec::new();
        for v in values {
            v.encode(&mut bytes);
        }
        assert_eq!(to_hex(&bytes), vector.bytes_hex);
    }
    // Sanity: the two f64 fields make 16 bytes per vector.
    assert!(rows.iter().all(|(_, v)| v.len() == 2));
    assert!(matches!(rows[0].1[0], ScalarValue::F64(_)));
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::new();
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
