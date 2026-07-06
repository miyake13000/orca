//! [`LayerStore`]: the committed-layer directory of one environment.

use std::path::PathBuf;

use orca_hash::{Hash, to_hex};

use crate::layer::{Layer, Upper};

/// Errors from [`LayerStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum LayerStoreError {
    /// A referenced layer directory does not exist.
    #[error("layer not found: {0}")]
    NotFound(String),
    /// Filesystem operation failed.
    #[error("layer store I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Store of committed layers under `envs/<uuid>/layers/<hash>/`.
///
/// Layer directory names are the immutable commit hashes; since hashes are
/// never recomputed, layers are never renamed after promotion.
pub struct LayerStore {
    base: PathBuf,
}

impl LayerStore {
    /// Create a store rooted at `envs/<uuid>/layers/`.
    pub fn new(base_path: &std::path::Path) -> Self {
        Self {
            base: base_path.to_path_buf(),
        }
    }

    /// Resolve layer hashes into [`Layer`]s, verifying each directory
    /// exists. Order is preserved (callers pass newest-first stacks).
    pub fn resolve(&self, hashes: &[Hash]) -> Result<Vec<Layer>, LayerStoreError> {
        hashes
            .iter()
            .map(|hash| {
                let path = self.base.join(to_hex(hash));
                if !path.is_dir() {
                    return Err(LayerStoreError::NotFound(to_hex(hash)));
                }
                Ok(Layer::new(path))
            })
            .collect()
    }

    /// Promote the upper (`diff/`) into a committed layer.
    ///
    /// Called after the commit hash is fixed: renames `diff/` to
    /// `layers/<hash>/` (O(1), same filesystem) and recreates an empty
    /// `diff/`. Side effects: two directory operations; no data is copied.
    pub fn promote_upper(&self, upper: &Upper, hash: &Hash) -> Result<Layer, LayerStoreError> {
        std::fs::create_dir_all(&self.base)?;
        let dest = self.base.join(to_hex(hash));
        std::fs::rename(upper.path(), &dest)?;
        std::fs::create_dir_all(upper.path())?;
        Ok(Layer::new(dest))
    }

    /// Delete the layer directory for `hash` (used by gc). Missing layers
    /// are ignored.
    pub fn delete(&self, hash: &Hash) -> Result<(), LayerStoreError> {
        let path = self.base.join(to_hex(hash));
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promote_resolve_delete() {
        let dir = tempfile::tempdir().unwrap();
        let layers = dir.path().join("layers");
        let diff = dir.path().join("diff");
        std::fs::create_dir_all(&diff).unwrap();
        std::fs::write(diff.join("file"), b"x").unwrap();

        let store = LayerStore::new(&layers);
        let upper = Upper::new(diff.clone());
        let hash: Hash = [7; 32];

        let layer = store.promote_upper(&upper, &hash).unwrap();
        assert!(layer.path().join("file").is_file());
        // diff/ is recreated empty.
        assert!(upper.is_empty().unwrap());

        let resolved = store.resolve(&[hash]).unwrap();
        assert_eq!(resolved[0].path(), layer.path());
        assert!(matches!(
            store.resolve(&[[9; 32]]),
            Err(LayerStoreError::NotFound(_))
        ));

        store.delete(&hash).unwrap();
        assert!(store.resolve(&[hash]).is_err());
        // Deleting again is a no-op.
        store.delete(&hash).unwrap();
    }
}
