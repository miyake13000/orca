//! [`LayerBlobStore`]: content-addressed store of extracted layer blobs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::layer::Layer;

use super::LayerDigest;

/// Errors from the blob store.
#[derive(Debug, thiserror::Error)]
pub enum BlobStoreError {
    /// A referenced blob directory does not exist.
    #[error("layer blob not found: {0}")]
    NotFound(String),
    /// Filesystem operation failed.
    #[error("blob store I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Content-addressed store of extracted OCI layers under
/// `images/layers/sha256/<digest>/`. Layers are shared between images and
/// garbage-collected against the index's reference set.
pub struct LayerBlobStore {
    base: PathBuf,
}

impl LayerBlobStore {
    /// Create a store rooted at `images/layers/sha256/`.
    pub fn new(base_path: &Path) -> Self {
        Self {
            base: base_path.to_path_buf(),
        }
    }

    /// Directory a digest maps to (may not exist yet).
    pub fn blob_path(&self, digest: &LayerDigest) -> PathBuf {
        self.base.join(digest.to_string())
    }

    /// Resolve digests into [`Layer`]s, verifying each directory exists.
    /// Order is preserved.
    pub fn resolve(&self, digests: &[LayerDigest]) -> Result<Vec<Layer>, BlobStoreError> {
        digests
            .iter()
            .map(|d| {
                let path = self.blob_path(d);
                if !path.is_dir() {
                    return Err(BlobStoreError::NotFound(d.to_string()));
                }
                Ok(Layer::new(path))
            })
            .collect()
    }

    /// Move an extracted layer directory into the store (rename; `src`
    /// must be on the same filesystem — the downloader extracts into a
    /// sibling temp directory to guarantee this). If the blob already
    /// exists, `src` is discarded.
    pub fn save(&self, digest: &LayerDigest, src: &Path) -> Result<(), BlobStoreError> {
        std::fs::create_dir_all(&self.base)?;
        let dest = self.blob_path(digest);
        if dest.exists() {
            std::fs::remove_dir_all(src)?;
            return Ok(());
        }
        std::fs::rename(src, dest)?;
        Ok(())
    }

    /// Whether the blob for `digest` is present.
    pub fn contains(&self, digest: &LayerDigest) -> bool {
        self.blob_path(digest).is_dir()
    }

    /// Delete every stored blob whose digest is not in `referenced`.
    pub fn gc(&self, referenced: &HashSet<LayerDigest>) -> Result<(), BlobStoreError> {
        let entries = match std::fs::read_dir(&self.base) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let referenced: HashSet<String> = referenced.iter().map(|d| d.to_string()).collect();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !referenced.contains(&name) {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_resolve_gc() {
        let dir = tempfile::tempdir().unwrap();
        let store = LayerBlobStore::new(&dir.path().join("sha256"));
        let digest = LayerDigest([5; 32]);

        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("file"), b"x").unwrap();
        store.save(&digest, &staging).unwrap();
        assert!(store.contains(&digest));
        assert!(store.resolve(&[digest]).is_ok());

        // gc with empty reference set removes it.
        store.gc(&HashSet::new()).unwrap();
        assert!(!store.contains(&digest));
        assert!(matches!(
            store.resolve(&[digest]),
            Err(BlobStoreError::NotFound(_))
        ));
    }
}
