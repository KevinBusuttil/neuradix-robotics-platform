//! External compilers are deliberate test prerequisites, not silent skips.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use neuradix_contracts::{Contract, load_file};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("neuradix-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn depth() -> Contract {
    load_file(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/standard/navigation/vehicle-depth.yaml"),
    )
    .unwrap()
}

pub fn tiny() -> Contract {
    load_file(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny-telemetry.yaml"))
        .unwrap()
}

pub fn require_compiler(candidates: &[&str]) -> String {
    candidates
        .iter()
        .find(|cc| {
            Command::new(cc)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .unwrap_or_else(|| panic!("required test compiler missing: {candidates:?}"))
        .to_string()
}

pub fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub fn run_cpp(dir: &TempDir, source: &str) {
    let compiler = require_compiler(&["g++", "c++", "clang++"]);
    let source_path = dir.0.join("main.cpp");
    let bin = dir.0.join("cpp-harness");
    std::fs::write(&source_path, source).unwrap();
    success(
        Command::new(compiler)
            .args(["-std=c++11", "-O2", "-Wall", "-Wextra", "-Werror"])
            .arg(&source_path)
            .arg("-o")
            .arg(&bin)
            .output()
            .unwrap(),
    );
    success(Command::new(bin).current_dir(&dir.0).output().unwrap());
}

pub fn run_rust(dir: &TempDir, source: &str) {
    let source_path = dir.0.join("main.rs");
    let bin = dir.0.join("rust-harness");
    std::fs::write(&source_path, source).unwrap();
    success(
        Command::new("rustc")
            .args(["--edition=2024", "-Dwarnings"])
            .arg(&source_path)
            .arg("-o")
            .arg(&bin)
            .output()
            .unwrap(),
    );
    success(Command::new(bin).current_dir(&dir.0).output().unwrap());
}
