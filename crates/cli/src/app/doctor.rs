//! The `doctor` command service: environment diagnostics.
//!
//! `doctor` never fails because an optional tool is missing; it reports what it
//! finds. It reports the selected Rust toolchain, workspace detection, the
//! supported contract format, writable output paths, the platform/architecture
//! and the availability of optional tools.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use neuradix_contracts::SUPPORTED_API_VERSION;

use crate::app::{AppError, Outcome};

/// `neuradix doctor`
pub fn run() -> Result<Outcome, AppError> {
    let workspace_root = find_workspace_root();
    let toolchain = selected_toolchain(workspace_root.as_deref());

    let data = json!({
        "rustToolchain": toolchain,
        "workspace": {
            "detected": workspace_root.is_some(),
            "root": workspace_root.as_ref().map(|p| p.display().to_string()),
        },
        "contractFormat": {
            "apiVersion": SUPPORTED_API_VERSION,
            "kinds": ["StreamContract"],
        },
        "writablePaths": {
            "tempDir": check_writable(&std::env::temp_dir()),
            "currentDir": std::env::current_dir()
                .map(|d| check_writable(&d))
                .unwrap_or(false),
        },
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
        },
        "optionalTools": {
            "rustc": tool_version("rustc"),
            "cargo": tool_version("cargo"),
            "git": tool_version("git"),
        },
    });

    let mut warnings = Vec::new();
    if workspace_root.is_none() {
        warnings.push("no Neuradix/Cargo workspace detected from the current directory".to_owned());
    }
    Ok(Outcome::with_warnings(data, warnings))
}

/// Walk upwards from the current directory looking for a Cargo workspace root.
fn find_workspace_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file()
            && std::fs::read_to_string(&manifest)
                .map(|c| c.contains("[workspace]"))
                .unwrap_or(false)
        {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Determine the selected toolchain: prefer the pinned `rust-toolchain.toml`
/// channel, otherwise fall back to the active `rustc` version.
fn selected_toolchain(workspace_root: Option<&Path>) -> Value {
    if let Some(root) = workspace_root {
        let pin = root.join("rust-toolchain.toml");
        if let Ok(contents) = std::fs::read_to_string(&pin)
            && let Some(channel) = parse_toolchain_channel(&contents)
        {
            return json!({ "source": "rust-toolchain.toml", "channel": channel });
        }
    }
    match tool_version("rustc") {
        Value::Null => json!({ "source": "unknown", "channel": Value::Null }),
        version => json!({ "source": "rustc", "channel": version }),
    }
}

/// Extract the `channel = "..."` value from a `rust-toolchain.toml`.
fn parse_toolchain_channel(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("channel") {
            let rest = rest.trim_start().strip_prefix('=')?.trim();
            let value = rest.trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// Return `<tool> --version` (first line) if the tool is available, else `null`.
fn tool_version(tool: &str) -> Value {
    match Command::new(tool).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let first = text.lines().next().unwrap_or("").trim().to_owned();
            Value::String(first)
        }
        _ => Value::Null,
    }
}

/// Whether a temporary file can be created and removed in `dir`.
fn check_writable(dir: &Path) -> bool {
    let probe = dir.join(".neuradix-doctor-write-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
