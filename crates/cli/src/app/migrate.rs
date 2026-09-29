//! `neuradix record migrate`: explicit, bounded, native-to-native migration of
//! legacy (c8aa467) declaration-order scalar payloads to `neuradix.scalar-le.v2`.
//!
//! All decoding, provenance validation and conversion is
//! `neuradix_record::legacy_scalar`; this module only bounds input, publishes
//! output safely and reports.
//!
//! ## Resource bounds
//!
//! - Recording: at most `--max-input-bytes` container bytes (default and
//!   ceiling 256 MiB) and `--max-records` records (default and ceiling 1 Mi),
//!   enforced while reading ([`NativeRecording::from_reader`]).
//! - Provenance: at most [`MAX_PROVENANCE_BYTES`] (1 MiB), enforced while
//!   reading; each embedded contract at most 64 KiB (library check).
//!
//! Peak memory is roughly two decoded copies of the recording: the read buffer
//! (≤ max + 1 bytes) is released after parsing, the source recording is dropped
//! after conversion, and the output is streamed to disk. Each decoded record
//! also costs a fixed-size [`neuradix_record::RawRecord`] plus its payload allocation, and the
//! record vectors may hold up to twice their length in spare capacity.
//!
//! ## Publication
//!
//! The source is never modified. The destination must not exist, and an
//! existing path that is the input (including a hard link to it) is refused as
//! in-place migration. Output is written to a new temporary file in the
//! destination directory, flushed and `fsync`ed, then published with
//! `link(2)`, which fails rather than replaces a destination created
//! concurrently. The temporary name is removed on success and on every error.
//! Durability of the directory entry across power loss is not claimed.

use std::fs::File;
use std::io::{BufWriter, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use neuradix_record::legacy_scalar::MAX_PROVENANCE_BYTES;
use neuradix_record::{
    LegacyProvenance, MAGIC, NativeReadLimits, NativeRecordWriter, NativeRecording, Recording,
    migrate_legacy_scalar,
};
use serde_json::{Value, json};

use crate::app::{AppError, Outcome};
use crate::exit::ExitCode;

fn fail(exit: ExitCode, message: impl Into<String>) -> AppError {
    AppError::message(exit, message)
}

/// `neuradix record migrate <file> --provenance <json> --out <file>`.
pub fn migrate(
    file: &Path,
    provenance: &Path,
    out: &Path,
    limits: NativeReadLimits,
) -> Result<Outcome, AppError> {
    let input = std::fs::metadata(file).map_err(|e| {
        fail(
            ExitCode::GeneralFailure,
            format!("could not read recording `{}`: {e}", file.display()),
        )
    })?;
    check_destination(out, &input)?;

    let recording = read_recording(file, limits)?;
    let request = read_provenance(provenance)?;
    let migrated = migrate_legacy_scalar(&recording, &request)
        .map_err(|e| fail(ExitCode::Compatibility, format!("migration refused: {e}")))?;
    let opaque: Vec<u16> = recording
        .manifest()
        .channels
        .iter()
        .map(|c| c.id)
        .filter(|id| {
            !migrated
                .report()
                .channels
                .iter()
                .any(|m| m.channel_id == *id)
        })
        .collect();
    drop(recording);

    let bytes = publish_new(out, |sink| {
        let mut writer = NativeRecordWriter::new(BufWriter::new(sink), migrated.manifest())
            .map_err(std::io::Error::other)?;
        for r in migrated.records() {
            writer
                .write_record(r.channel_id, r.sequence, r.timestamp, &r.payload)
                .map_err(std::io::Error::other)?;
        }
        writer
            .finish()
            .map_err(std::io::Error::other)?
            .into_inner()
            .map_err(|e| e.into_error())?;
        Ok(())
    })
    .map_err(|e| {
        fail(
            ExitCode::GeneralFailure,
            format!("could not publish `{}`: {e}", out.display()),
        )
    })?;

    let report = migrated.report();
    let channels: Vec<Value> = report
        .channels
        .iter()
        .map(|c| {
            json!({
                "channelId": c.channel_id,
                "records": c.records,
                "legacyWireLen": c.legacy_wire_len,
                "codecId": c.codec_id,
                "wireId": c.wire_id,
            })
        })
        .collect();
    Ok(Outcome::new(json!({
        "source": file.display().to_string(),
        "file": out.display().to_string(),
        "format": "native",
        "formatVersion": migrated.manifest().format_version,
        "bytes": bytes,
        "records": migrated.records().len(),
        "sourceCodec": report.source_codec,
        "sourceRevision": report.source_revision,
        "sourceDigest": report.source_digest,
        "migratedDigest": report.migrated_digest,
        "channels": channels,
        "opaqueChannels": opaque,
        "limits": {
            "maxInputBytes": limits.max_bytes(),
            "maxRecords": limits.max_records(),
            "maxProvenanceBytes": MAX_PROVENANCE_BYTES,
        },
    })))
}

/// Refuse in-place migration and any existing destination (including dangling
/// symlinks), before reading anything large.
fn check_destination(out: &Path, input: &std::fs::Metadata) -> Result<(), AppError> {
    if out.file_name().is_none() {
        return Err(fail(
            ExitCode::InvalidUse,
            format!("destination `{}` has no file name", out.display()),
        ));
    }
    match std::fs::symlink_metadata(out) {
        Ok(_) => {
            let same = std::fs::metadata(out)
                .is_ok_and(|m| m.dev() == input.dev() && m.ino() == input.ino());
            Err(fail(
                ExitCode::InvalidUse,
                if same {
                    format!(
                        "refusing in-place migration: `{}` is the input recording",
                        out.display()
                    )
                } else {
                    format!("refusing to overwrite existing `{}`", out.display())
                },
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let dir = parent(out);
            if std::fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
                Ok(())
            } else {
                Err(fail(
                    ExitCode::GeneralFailure,
                    format!("destination directory `{}` does not exist", dir.display()),
                ))
            }
        }
        Err(e) => Err(fail(
            ExitCode::GeneralFailure,
            format!("could not check destination `{}`: {e}", out.display()),
        )),
    }
}

fn parent(out: &Path) -> PathBuf {
    match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn read_recording(file: &Path, limits: NativeReadLimits) -> Result<NativeRecording, AppError> {
    let invalid = |e: String| {
        fail(
            ExitCode::GeneralFailure,
            format!("invalid recording `{}`: {e}", file.display()),
        )
    };
    let mut input = File::open(file).map_err(|e| invalid(e.to_string()))?;
    let mut magic = [0u8; MAGIC.len()];
    if input.read_exact(&mut magic).is_err() || magic != MAGIC {
        return Err(invalid(
            "`record migrate` reads native (.nrec) recordings only".into(),
        ));
    }
    let prefixed = std::io::Cursor::new(magic).chain(input);
    NativeRecording::from_reader(prefixed, limits).map_err(|e| invalid(e.to_string()))
}

fn read_provenance(path: &Path) -> Result<LegacyProvenance, AppError> {
    let io = |e: std::io::Error| {
        fail(
            ExitCode::GeneralFailure,
            format!("could not read provenance `{}`: {e}", path.display()),
        )
    };
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io)?
        .take(MAX_PROVENANCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > MAX_PROVENANCE_BYTES {
        return Err(fail(
            ExitCode::GeneralFailure,
            format!(
                "provenance `{}` exceeds {MAX_PROVENANCE_BYTES} bytes",
                path.display()
            ),
        ));
    }
    LegacyProvenance::from_json(&bytes)
        .map_err(|e| fail(ExitCode::Compatibility, format!("migration refused: {e}")))
}

/// Temporary file in the destination directory, removed unless disarmed.
struct Partial(Option<PathBuf>);
impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Write a new file at `out` without ever replacing an existing one: write a
/// fresh temporary sibling, `fsync` it, then `link(2)` it into place (which
/// fails with `AlreadyExists` if `out` appeared meanwhile) and remove the
/// temporary name. Returns the published size. On error nothing is left
/// behind: no destination and no temporary file.
pub(crate) fn publish_new(
    out: &Path,
    write: impl FnOnce(&File) -> std::io::Result<()>,
) -> std::io::Result<u64> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = parent(out);
    let name = out
        .file_name()
        .ok_or_else(|| std::io::Error::other("destination has no file name"))?
        .to_string_lossy()
        .into_owned();
    let mut created = None;
    for _ in 0..16 {
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!(
            ".{name}.neuradix-migrate-{}-{id}.partial",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                created = Some((Partial(Some(path)), file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let (partial, file) =
        created.ok_or_else(|| std::io::Error::other("temporary-name admission exhausted"))?;
    write(&file)?;
    file.sync_all()?;
    let bytes = file.metadata()?.len();
    drop(file);
    std::fs::hard_link(partial.0.as_ref().expect("armed"), out)?;
    // `partial` drops here and removes the temporary name; `out` remains.
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neuradix-migrate-unit-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        dir
    }
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn publish_writes_then_links_and_leaves_no_temporary() {
        let dir = scratch("ok");
        let out = dir.join("out.nrec");
        let n = publish_new(&out, |mut f| f.write_all(b"hello")).unwrap();
        assert_eq!(n, 5);
        assert_eq!(std::fs::read(&out).unwrap(), b"hello");
        assert_eq!(entries(&dir), ["out.nrec"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_failure_leaves_no_destination_or_temporary() {
        let dir = scratch("fail");
        let out = dir.join("out.nrec");
        let err = publish_new(&out, |mut f| {
            f.write_all(b"partial")?;
            Err(std::io::Error::other("injected write failure"))
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "injected write failure");
        assert!(entries(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrently_created_destination_is_not_replaced() {
        let dir = scratch("race");
        let out = dir.join("out.nrec");
        let err = publish_new(&out, |mut f| {
            // Another writer creates the destination before publication.
            std::fs::write(&out, b"theirs")?;
            f.write_all(b"ours")
        })
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&out).unwrap(), b"theirs");
        assert_eq!(entries(&dir), ["out.nrec"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_directory_fails_before_writing() {
        let dir = scratch("missing");
        let out = dir.join("absent/out.nrec");
        let mut called = false;
        assert!(
            publish_new(&out, |_| {
                called = true;
                Ok(())
            })
            .is_err()
        );
        assert!(!called);
        assert!(entries(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
