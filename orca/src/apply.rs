//! Journaled application of [`Change`]s to the host filesystem.
//!
//! A full transaction over the live `/` is impossible, so `apply` settles
//! for per-file atomic replacement plus an undo journal (DESIGN §6):
//!
//! 1. the **manifest** (all changes) is written to `apply-journal/`;
//! 2. an **undo journal** records the inverse of every change — backups of
//!    files to be modified/deleted are taken *before* anything mutates;
//! 3. changes execute in order; `Create`/`Modify` write a temp file next
//!    to the target (same filesystem) and `rename(2)` it into place; a
//!    progress marker advances after each step;
//! 4. on success the journal is discarded.
//!
//! On failure or interruption the undo journal replays in reverse to
//! restore the host. A leftover journal is detected at the next apply and
//! can be rolled back or resumed ([`rollback`] / [`resume`]).
//!
//! Ownership, mode and xattrs are preserved when installing files.
//! Fifos / sockets / device nodes are skipped with a warning (initial
//! scope; DESIGN §6).

use std::path::{Path, PathBuf};

use orca_image::Change;
use serde::{Deserialize, Serialize};

/// Errors from applying changes to the host.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    /// Filesystem operation failed.
    #[error("apply failed at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The journal could not be (de)serialized.
    #[error("apply journal error: {0}")]
    Journal(#[from] serde_json::Error),
    /// No pending journal exists to resume or roll back.
    #[error("no pending apply journal")]
    NoJournal,
    /// A change failed and the automatic rollback also failed — the host
    /// may be partially modified; the journal is kept for manual retry.
    #[error("apply failed ({apply}) and rollback also failed ({rollback}); journal kept at {journal}")]
    RollbackFailed {
        /// The original apply error.
        apply: String,
        /// The rollback error.
        rollback: String,
        /// Journal location for manual recovery.
        journal: PathBuf,
    },
}

fn io_err(path: &Path, source: std::io::Error) -> ApplyError {
    ApplyError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The inverse of one change, recorded before any mutation.
#[derive(Debug, Serialize, Deserialize)]
enum UndoOp {
    /// The change created `path`; undo removes it.
    RemoveCreated {
        /// Absolute target path.
        path: PathBuf,
    },
    /// The change overwrote/deleted `path`; undo restores the backup.
    Restore {
        /// Absolute target path.
        path: PathBuf,
        /// Backup location inside the journal.
        backup: PathBuf,
    },
    /// Nothing to undo (e.g. delete of an already-missing path).
    None,
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}
fn undo_path(dir: &Path) -> PathBuf {
    dir.join("undo.json")
}
fn done_path(dir: &Path) -> PathBuf {
    dir.join("done")
}

/// Whether an unfinished apply journal exists in `dir`.
pub fn has_pending(dir: &Path) -> bool {
    manifest_path(dir).exists()
}

/// Execute `changes` against the host, journaling into `dir`.
///
/// On any failure the undo journal is replayed automatically; if that
/// rollback itself fails, the journal is kept and
/// [`ApplyError::RollbackFailed`] describes both errors.
pub fn execute(changes: &[Change], dir: &Path) -> Result<(), ApplyError> {
    std::fs::create_dir_all(dir.join("backup")).map_err(|e| io_err(dir, e))?;
    let manifest = serde_json::to_vec_pretty(changes)?;
    std::fs::write(manifest_path(dir), manifest).map_err(|e| io_err(dir, e))?;

    // Record undo ops (taking backups) before mutating anything.
    let mut undo = Vec::with_capacity(changes.len());
    for (i, change) in changes.iter().enumerate() {
        undo.push(record_undo(dir, i, change)?);
    }
    std::fs::write(undo_path(dir), serde_json::to_vec_pretty(&undo)?)
        .map_err(|e| io_err(dir, e))?;
    write_done(dir, -1)?;

    match run_changes(changes, dir, 0) {
        Ok(()) => {
            cleanup(dir);
            Ok(())
        }
        Err(apply_err) => match rollback(dir) {
            Ok(()) => Err(apply_err),
            Err(rb_err) => Err(ApplyError::RollbackFailed {
                apply: apply_err.to_string(),
                rollback: rb_err.to_string(),
                journal: dir.to_path_buf(),
            }),
        },
    }
}

/// Resume a previously interrupted apply from its journal: re-runs the
/// remaining changes (per-file replacement is idempotent).
pub fn resume(dir: &Path) -> Result<(), ApplyError> {
    let changes = load_manifest(dir)?;
    let done = read_done(dir);
    run_changes(&changes, dir, (done + 1) as usize)?;
    cleanup(dir);
    Ok(())
}

/// Roll back an interrupted apply: replay the undo journal in reverse for
/// every executed change, then discard the journal.
pub fn rollback(dir: &Path) -> Result<(), ApplyError> {
    if !has_pending(dir) {
        return Err(ApplyError::NoJournal);
    }
    let undo: Vec<UndoOp> = serde_json::from_slice(
        &std::fs::read(undo_path(dir)).map_err(|e| io_err(dir, e))?,
    )?;
    let done = read_done(dir);
    for i in (0..=done).rev() {
        let Some(op) = undo.get(i as usize) else {
            continue;
        };
        match op {
            UndoOp::RemoveCreated { path } => remove_any(path)?,
            UndoOp::Restore { path, backup } => {
                remove_any(path)?;
                copy_preserving(backup, path)?;
            }
            UndoOp::None => {}
        }
    }
    cleanup(dir);
    Ok(())
}

/// Load the pending manifest (for display before resume/rollback).
pub fn load_manifest(dir: &Path) -> Result<Vec<Change>, ApplyError> {
    if !has_pending(dir) {
        return Err(ApplyError::NoJournal);
    }
    Ok(serde_json::from_slice(
        &std::fs::read(manifest_path(dir)).map_err(|e| io_err(dir, e))?,
    )?)
}

fn run_changes(changes: &[Change], dir: &Path, start: usize) -> Result<(), ApplyError> {
    for (i, change) in changes.iter().enumerate().skip(start) {
        perform(change)?;
        write_done(dir, i as i64)?;
    }
    Ok(())
}

fn record_undo(dir: &Path, index: usize, change: &Change) -> Result<UndoOp, ApplyError> {
    let target = host_path(change.path());
    let exists = target.symlink_metadata().is_ok();
    match change {
        Change::Create { .. } | Change::Modify { .. } => {
            if exists {
                let backup = dir.join("backup").join(index.to_string());
                copy_preserving(&target, &backup)?;
                Ok(UndoOp::Restore {
                    path: target,
                    backup,
                })
            } else {
                Ok(UndoOp::RemoveCreated { path: target })
            }
        }
        Change::Delete { .. } => {
            if exists {
                let backup = dir.join("backup").join(index.to_string());
                copy_preserving(&target, &backup)?;
                Ok(UndoOp::Restore {
                    path: target,
                    backup,
                })
            } else {
                Ok(UndoOp::None)
            }
        }
    }
}

/// Absolute host path for a root-relative change path.
fn host_path(rel: &Path) -> PathBuf {
    Path::new("/").join(rel)
}

/// Execute one change on the host.
fn perform(change: &Change) -> Result<(), ApplyError> {
    match change {
        Change::Create { path, source } | Change::Modify { path, source } => {
            install(source, &host_path(path))
        }
        Change::Delete { path } => remove_any(&host_path(path)),
    }
}

/// Remove a path of any kind; missing is fine.
fn remove_any(target: &Path) -> Result<(), ApplyError> {
    let md = match target.symlink_metadata() {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_err(target, e)),
    };
    let result = if md.is_dir() {
        std::fs::remove_dir_all(target)
    } else {
        std::fs::remove_file(target)
    };
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(target, e)),
    }
}

/// Install `source` (a file, directory or symlink inside a layer) at
/// `target` with atomic replacement for files/symlinks: write a temp name
/// in the target's directory, then `rename(2)` over.
///
/// Directories are created in place (no rename) and have their metadata
/// applied; fifos/sockets/devices are skipped with a warning.
fn install(source: &Path, target: &Path) -> Result<(), ApplyError> {
    let md = source.symlink_metadata().map_err(|e| io_err(source, e))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }

    let ft = md.file_type();
    if ft.is_dir() {
        // Replace a non-dir in the way, then create + apply metadata.
        if let Ok(t) = target.symlink_metadata()
            && !t.is_dir()
        {
            remove_any(target)?;
        }
        std::fs::create_dir_all(target).map_err(|e| io_err(target, e))?;
        apply_metadata(source, target, &md)?;
        return Ok(());
    }

    use std::os::unix::fs::FileTypeExt;
    if ft.is_fifo() || ft.is_socket() || ft.is_block_device() || ft.is_char_device() {
        eprintln!(
            "orca: warning: skipping special file {} (not supported by apply)",
            target.display()
        );
        return Ok(());
    }

    let tmp = tmp_name(target);
    let _ = std::fs::remove_file(&tmp);
    if ft.is_symlink() {
        let link = std::fs::read_link(source).map_err(|e| io_err(source, e))?;
        std::os::unix::fs::symlink(&link, &tmp).map_err(|e| io_err(&tmp, e))?;
        use std::os::unix::fs::MetadataExt;
        let _ = std::os::unix::fs::lchown(&tmp, Some(md.uid()), Some(md.gid()));
    } else {
        std::fs::copy(source, &tmp).map_err(|e| io_err(source, e))?;
        apply_metadata(source, &tmp, &md)?;
    }
    // rename cannot replace a directory; clear one out of the way first.
    if let Ok(t) = target.symlink_metadata()
        && t.is_dir()
    {
        remove_any(target)?;
    }
    std::fs::rename(&tmp, target).map_err(|e| io_err(target, e))?;
    Ok(())
}

/// Temp name in the same directory (same filesystem) as `target`.
fn tmp_name(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    target.with_file_name(format!(".orca-tmp-{}-{file_name}", std::process::id()))
}

/// Copy mode, ownership and xattrs from `source`(+`md`) onto `dest`
/// (which must not be a symlink). xattr copy failures are ignored — not
/// all filesystems support them.
fn apply_metadata(
    source: &Path,
    dest: &Path,
    md: &std::fs::Metadata,
) -> Result<(), ApplyError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(md.mode()))
        .map_err(|e| io_err(dest, e))?;
    std::os::unix::fs::chown(dest, Some(md.uid()), Some(md.gid()))
        .map_err(|e| io_err(dest, e))?;
    if let Ok(attrs) = xattr::list(source) {
        for attr in attrs {
            if let Ok(Some(value)) = xattr::get(source, &attr) {
                let _ = xattr::set(dest, &attr, &value);
            }
        }
    }
    Ok(())
}

/// Recursively copy `src` to `dst`, preserving type, mode, ownership and
/// xattrs. Used for undo backups and their restoration (backups may cross
/// filesystems, so rename is not an option).
fn copy_preserving(src: &Path, dst: &Path) -> Result<(), ApplyError> {
    let md = src.symlink_metadata().map_err(|e| io_err(src, e))?;
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    let ft = md.file_type();
    if ft.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| io_err(dst, e))?;
        for entry in std::fs::read_dir(src).map_err(|e| io_err(src, e))? {
            let entry = entry.map_err(|e| io_err(src, e))?;
            copy_preserving(&entry.path(), &dst.join(entry.file_name()))?;
        }
        apply_metadata(src, dst, &md)?;
    } else if ft.is_symlink() {
        let link = std::fs::read_link(src).map_err(|e| io_err(src, e))?;
        let _ = std::fs::remove_file(dst);
        std::os::unix::fs::symlink(&link, dst).map_err(|e| io_err(dst, e))?;
        use std::os::unix::fs::MetadataExt;
        let _ = std::os::unix::fs::lchown(dst, Some(md.uid()), Some(md.gid()));
    } else if ft.is_file() {
        std::fs::copy(src, dst).map_err(|e| io_err(src, e))?;
        apply_metadata(src, dst, &md)?;
    }
    // Special files are not backed up (apply skips them too).
    Ok(())
}

fn write_done(dir: &Path, index: i64) -> Result<(), ApplyError> {
    std::fs::write(done_path(dir), index.to_string()).map_err(|e| io_err(dir, e))
}

fn read_done(dir: &Path) -> i64 {
    std::fs::read_to_string(done_path(dir))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1)
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive execute() against a fake "host" by using relative paths...
    /// not possible: apply targets absolute "/". Instead we test the
    /// journal helpers and copy_preserving; end-to-end apply is covered
    /// by the root-only integration test in the workspace crate.
    #[test]
    fn copy_preserving_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/file"), b"hello").unwrap();
        std::os::unix::fs::symlink("sub/file", src.join("link")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            &src.join("sub/file"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();

        let dst = dir.path().join("dst");
        copy_preserving(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("sub/file")).unwrap(), b"hello");
        assert_eq!(
            std::fs::read_link(dst.join("link")).unwrap(),
            PathBuf::from("sub/file")
        );
        let mode = std::fs::metadata(dst.join("sub/file"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    #[test]
    fn done_marker_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_done(dir.path()), -1);
        write_done(dir.path(), 5).unwrap();
        assert_eq!(read_done(dir.path()), 5);
    }

    #[test]
    fn pending_detection() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_pending(dir.path()));
        std::fs::write(manifest_path(dir.path()), b"[]").unwrap();
        assert!(has_pending(dir.path()));
        assert!(load_manifest(dir.path()).unwrap().is_empty());
    }
}
