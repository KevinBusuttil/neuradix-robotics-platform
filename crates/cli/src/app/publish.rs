//! Shared bounded input reads and no-overwrite output publication for CLI
//! commands that create artifacts (`record migrate`, `channel manifest`,
//! `channel table`).
//!
//! ## Publication
//!
//! [`publish_new`] writes a new temporary sibling of the destination (created
//! with `create_new`), flushes and `fsync`s it, then publishes it with
//! `link(2)`. `link` fails with `AlreadyExists` rather than replacing a
//! destination created concurrently. The temporary name is then removed.
//!
//! Temporary cleanup is best effort. Removal is attempted on success and on
//! every error; if the final removal after a successful publication fails, the
//! leftover path is returned so the caller can report it. A removal failure on
//! an error path, or a process killed mid-write, can leave a
//! `.<name>.neuradix-<tag>-<pid>-<n>.partial` file. A partial *destination* is
//! never left behind. The directory is not `fsync`ed, so durability of the new
//! entry across power loss is not claimed. Filesystems without hard links
//! cannot publish, and fail cleanly.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Directory that holds `out` (`.` for a bare file name).
pub(crate) fn parent(out: &Path) -> PathBuf {
    match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Why a bounded read failed.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// Opening or reading failed.
    Io(std::io::Error),
    /// The input has more than `limit` bytes; at most `limit + 1` were read.
    TooLarge { limit: u64 },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(e) => e.fmt(f),
            ReadError::TooLarge { limit } => write!(f, "exceeds {limit} bytes"),
        }
    }
}

/// Read at most `limit` bytes from `path`. The limit is enforced while
/// reading (`take(limit + 1)`), never after an unbounded read.
pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ReadError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(ReadError::Io)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > limit {
        return Err(ReadError::TooLarge { limit });
    }
    Ok(bytes)
}

/// A completed publication.
#[derive(Debug)]
pub(crate) struct Published {
    /// Size of the published file.
    pub bytes: u64,
    /// Temporary name that could not be removed after publication, if any.
    pub leftover: Option<PathBuf>,
}

/// A warning when a temporary file could not be removed after publication.
pub(crate) fn leftover_warning(published: &Published) -> Vec<String> {
    published
        .leftover
        .iter()
        .map(|p| {
            format!(
                "published, but could not remove temporary `{}`",
                p.display()
            )
        })
        .collect()
}

/// Temporary file in the destination directory, removed on drop unless taken.
struct Partial(Option<PathBuf>);
impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            // Best effort on error paths; see the module documentation.
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Write a new file at `out` without ever replacing an existing one. `tag`
/// names the command in the temporary file name. On error no destination is
/// created and removal of the temporary file is attempted.
pub(crate) fn publish_new(
    out: &Path,
    tag: &str,
    write: impl FnOnce(&File) -> std::io::Result<()>,
) -> std::io::Result<Published> {
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
            ".{name}.neuradix-{tag}-{}-{id}.partial",
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
    let (mut partial, file) =
        created.ok_or_else(|| std::io::Error::other("temporary-name admission exhausted"))?;
    write(&file)?;
    file.sync_all()?;
    let bytes = file.metadata()?.len();
    drop(file);
    std::fs::hard_link(partial.0.as_ref().expect("armed"), out)?;
    let temporary = partial.0.take().expect("armed");
    let leftover = std::fs::remove_file(&temporary).err().map(|_| temporary);
    Ok(Published { bytes, leftover })
}

/// Why a destination was not accepted.
#[derive(Debug)]
pub(crate) enum DestinationError {
    /// The destination is unusable as given: it exists (possibly as one of the
    /// inputs) or has no file name. Callers report this as invalid use.
    Refused(String),
    /// Checking the destination failed (permission denied, a symlink loop, a
    /// path component that is not a directory, ...). An operational failure.
    Io(String),
}

impl std::fmt::Display for DestinationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DestinationError::Refused(m) | DestinationError::Io(m) => f.write_str(m),
        }
    }
}

/// Refuse any existing path at `out` (file, directory, symlink or dangling
/// symlink). `inputs` are paths this command reads; an existing `out` naming
/// one of them is reported as an attempt to overwrite an input. Errors other
/// than `NotFound` while checking are [`DestinationError::Io`].
pub(crate) fn check_new_destination(out: &Path, inputs: &[&Path]) -> Result<(), DestinationError> {
    use std::os::unix::fs::MetadataExt;
    if out.file_name().is_none() {
        return Err(DestinationError::Refused(format!(
            "destination `{}` has no file name",
            out.display()
        )));
    }
    match std::fs::symlink_metadata(out) {
        Ok(_) => {
            let target = std::fs::metadata(out).ok();
            let is_input = target.is_some_and(|t| {
                inputs.iter().any(|i| {
                    std::fs::metadata(i).is_ok_and(|m| m.dev() == t.dev() && m.ino() == t.ino())
                })
            });
            Err(DestinationError::Refused(if is_input {
                format!("refusing to overwrite input `{}`", out.display())
            } else {
                format!("refusing to overwrite existing `{}`", out.display())
            }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(DestinationError::Io(format!(
            "could not check destination `{}`: {e}",
            out.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "neuradix-publish-unit-{}-{label}",
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
        let published = publish_new(&out, "test", |mut f| f.write_all(b"hello")).unwrap();
        assert_eq!(published.bytes, 5);
        assert!(published.leftover.is_none());
        assert_eq!(std::fs::read(&out).unwrap(), b"hello");
        assert_eq!(entries(&dir), ["out.nrec"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_failure_leaves_no_destination_or_temporary() {
        let dir = scratch("fail");
        let out = dir.join("out.nrec");
        let err = publish_new(&out, "test", |mut f| {
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
        let err = publish_new(&out, "test", |mut f| {
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
    fn bounded_reads_stop_one_byte_past_the_limit() {
        let dir = scratch("read");
        let file = dir.join("in");
        std::fs::write(&file, vec![7u8; 100]).unwrap();
        assert_eq!(read_bounded(&file, 100).unwrap().len(), 100);
        assert!(matches!(
            read_bounded(&file, 99),
            Err(ReadError::TooLarge { limit: 99 })
        ));
        assert!(matches!(
            read_bounded(&dir.join("absent"), 10),
            Err(ReadError::Io(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_destinations_and_inputs_are_refused() {
        let dir = scratch("dest");
        let input = dir.join("in");
        std::fs::write(&input, b"x").unwrap();
        assert!(check_new_destination(&dir.join("new"), &[&input]).is_ok());
        assert!(
            check_new_destination(&input, &[&input])
                .unwrap_err()
                .to_string()
                .contains("overwrite input")
        );
        std::os::unix::fs::symlink(&input, dir.join("link")).unwrap();
        assert!(
            check_new_destination(&dir.join("link"), &[&input])
                .unwrap_err()
                .to_string()
                .contains("overwrite input")
        );
        std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("dangling")).unwrap();
        assert!(
            check_new_destination(&dir.join("dangling"), &[&input])
                .unwrap_err()
                .to_string()
                .contains("existing")
        );
        assert!(matches!(
            check_new_destination(&input.join("child"), &[&input]),
            Err(DestinationError::Io(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_directory_fails_before_writing() {
        let dir = scratch("missing");
        let out = dir.join("absent/out.nrec");
        let mut called = false;
        assert!(
            publish_new(&out, "test", |_| {
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
