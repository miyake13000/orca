//! [`Layer`], [`Upper`] and the deterministic tree hash used for commits.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use orca_hash::{Hashable, Hasher};

/// A committed (read-only) layer directory.
///
/// Invariant: `path` names an existing directory containing an OverlayFS
/// layer (`envs/<uuid>/layers/<hash>/`, an extracted OCI blob, or `/` when
/// used as the host baseline in diff stacks).
#[derive(Debug, Clone)]
pub struct Layer {
    pub(crate) path: PathBuf,
}

impl Layer {
    /// Wrap a layer directory path. Also used to treat the host rootfs
    /// (`/`) as the bottom layer of a diff stack.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The layer directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The writable upper directory of an environment (`envs/<uuid>/diff/`),
/// holding uncommitted changes.
#[derive(Debug, Clone)]
pub struct Upper {
    pub(crate) path: PathBuf,
}

impl Upper {
    /// Wrap the upper directory path (the `orca` crate builds this from
    /// `Env::upper_path()`).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The upper directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the upper contains no entries (clean working state).
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error if the directory cannot be read.
    pub fn is_empty(&self) -> std::io::Result<bool> {
        Ok(std::fs::read_dir(&self.path)?.next().is_none())
    }
}

/// Hash the upper's tree (paths + metadata, no file contents).
///
/// # Panics
///
/// Panics on I/O errors while walking the tree (orca runs as root, so this
/// only happens on hardware / filesystem failures).
impl Hashable for Upper {
    fn hash(&self, hasher: &mut dyn Hasher) {
        hash_dir(&self.path, hasher);
    }
}

/// Hash the layer's tree (paths + metadata, no file contents).
///
/// # Panics
///
/// Same conditions as the [`Upper`] implementation.
impl Hashable for Layer {
    fn hash(&self, hasher: &mut dyn Hasher) {
        hash_dir(&self.path, hasher);
    }
}

/// Walk `path` in a deterministic order and stream each entry's identity
/// into `hasher`.
///
/// Hashed metadata per entry (DESIGN §6): relative path, file type, mode,
/// uid/gid, size (regular files only), symlink target, and rdev (device
/// nodes, which is how whiteouts enter the hash). mtime is deliberately
/// excluded so copies hash identically. File contents are not read — the
/// hash identifies tree structure, not content (dedup/integrity are out of
/// scope; extend here if they are ever added).
///
/// # Panics
///
/// Panics on I/O errors during the walk (unreadable entries). The target
/// is the environment's own `diff/`, owned by root, so this indicates a
/// broken filesystem rather than a recoverable condition.
fn hash_dir(path: &Path, hasher: &mut dyn Hasher) {
    let walker = walkdir::WalkDir::new(path)
        .min_depth(1)
        .follow_links(false)
        .sort_by_file_name();
    for entry in walker {
        let entry = entry.expect("failed to walk layer directory while hashing");
        let rel = entry
            .path()
            .strip_prefix(path)
            .expect("walkdir yields children of the root");
        let md = entry
            .metadata()
            .expect("failed to stat layer entry while hashing");

        hasher.update_framed(rel.as_os_str().as_bytes());
        hasher.update(&file_type_tag(&md).to_le_bytes());
        hasher.update(&md.mode().to_le_bytes());
        hasher.update(&md.uid().to_le_bytes());
        hasher.update(&md.gid().to_le_bytes());
        let size = if md.is_file() { md.size() } else { 0 };
        hasher.update(&size.to_le_bytes());
        let target = if md.file_type().is_symlink() {
            std::fs::read_link(entry.path())
                .expect("failed to read symlink target while hashing")
                .into_os_string()
        } else {
            Default::default()
        };
        hasher.update_framed(target.as_bytes());
        hasher.update(&md.rdev().to_le_bytes());
    }
}

/// Stable single-byte tag for the entry's file type.
fn file_type_tag(md: &std::fs::Metadata) -> u8 {
    let ft = md.file_type();
    use std::os::unix::fs::FileTypeExt;
    if ft.is_dir() {
        b'd'
    } else if ft.is_symlink() {
        b'l'
    } else if ft.is_char_device() {
        b'c'
    } else if ft.is_block_device() {
        b'b'
    } else if ft.is_fifo() {
        b'p'
    } else if ft.is_socket() {
        b's'
    } else {
        b'f'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orca_hash::Sha256Hasher;

    fn hash_of(path: &Path) -> orca_hash::Hash {
        let mut h = Sha256Hasher::new();
        hash_dir(path, &mut h);
        h.finalize()
    }

    #[test]
    fn empty_dirs_hash_equal() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(hash_of(a.path()), hash_of(b.path()));
    }

    #[test]
    fn structure_changes_hash() {
        let dir = tempfile::tempdir().unwrap();
        let empty = hash_of(dir.path());

        std::fs::create_dir(dir.path().join("etc")).unwrap();
        std::fs::write(dir.path().join("etc/foo"), b"x").unwrap();
        let with_file = hash_of(dir.path());
        assert_ne!(empty, with_file);

        // Same tree in another location hashes identically (relative paths).
        let copy = tempfile::tempdir().unwrap();
        std::fs::create_dir(copy.path().join("etc")).unwrap();
        std::fs::write(copy.path().join("etc/foo"), b"x").unwrap();
        assert_eq!(with_file, hash_of(copy.path()));

        // Content size participates; mtime does not.
        std::fs::write(copy.path().join("etc/foo"), b"xy").unwrap();
        assert_ne!(with_file, hash_of(copy.path()));
    }

    #[test]
    fn symlink_target_participates() {
        let a = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("target-a", a.path().join("link")).unwrap();
        let b = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("target-b", b.path().join("link")).unwrap();
        assert_ne!(hash_of(a.path()), hash_of(b.path()));
    }
}
