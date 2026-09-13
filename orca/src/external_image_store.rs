//! [`ExternalImageStore`]: the shared pulled-image area (`images/`).
//!
//! Wires caller-supplied index and blob paths to the mechanisms provided
//! by `orca-image` ([`ImageIndex`], [`LayerBlobStore`], [`Downloader`]).
//! The normal layout is chosen by Orca, not inferred from a data root here.

use std::path::{Path, PathBuf};

use orca_image::Layer;
use orca_image::external_image::{
    BlobStoreError, Downloader, ImageDigest, ImageIndex, ImageManifest, IndexError, LayerBlobStore,
    PullError,
};

/// Errors from image store operations.
#[derive(Debug, thiserror::Error)]
pub enum ExternalImageError {
    /// Image index error (lookup, parse, I/O).
    #[error(transparent)]
    Index(#[from] IndexError),
    /// Blob store error.
    #[error(transparent)]
    Blob(#[from] BlobStoreError),
    /// Registry download error.
    #[error(transparent)]
    Pull(#[from] PullError),
}

/// The shared image area under the orca data root: the `images.toml`
/// index plus the extracted-blob CAS (`images/layers/sha256/`).
#[derive(Debug, Clone, Default)]
pub struct ExternalImageStore {
    index_file: PathBuf,
    blobs_dir: PathBuf,
}

impl ExternalImageStore {
    /// Bind independently supplied metadata and layer directories. No I/O.
    pub fn new(index_file: &Path, blobs_dir: &Path) -> Self {
        Self {
            index_file: index_file.into(),
            blobs_dir: blobs_dir.into(),
        }
    }

    fn index(&self) -> Result<ImageIndex, ExternalImageError> {
        Ok(ImageIndex::load(&self.index_file)?)
    }

    /// The extracted-blob CAS directory (`images/layers/sha256/`).
    pub(crate) fn blobs_dir(&self) -> PathBuf {
        self.blobs_dir.clone()
    }

    fn blobs(&self) -> LayerBlobStore {
        LayerBlobStore::new(&self.blobs_dir())
    }

    /// Look up a pulled image by reference (`ubuntu:24.04`); `None` if it
    /// has not been pulled yet.
    pub fn find(&self, reference: &str) -> Result<Option<ImageDigest>, ExternalImageError> {
        Ok(self.index()?.resolve(reference).ok().map(|m| m.digest))
    }

    /// Pull an image from its registry into the blob CAS and record it in
    /// the index. Blocks for the whole download.
    pub fn pull(&self, reference: &str) -> Result<ImageManifest, ExternalImageError> {
        let mut index = self.index()?;
        let manifest = Downloader::pull(reference, &self.blobs())?;
        index.insert(manifest.clone());
        index.save()?;
        Ok(manifest)
    }

    /// Drop an image record, then prune blobs no image references anymore
    /// (layers are shared, so deletion is reference-set based).
    pub fn remove(&self, reference: &str) -> Result<ImageManifest, ExternalImageError> {
        let mut index = self.index()?;
        let removed = index.remove(reference)?;
        index.save()?;
        self.blobs().gc(&index.referenced_layers())?;
        Ok(removed)
    }

    /// All pulled images, in index order.
    pub fn list(&self) -> Result<Vec<ImageManifest>, ExternalImageError> {
        Ok(self.index()?.list().into_iter().cloned().collect())
    }

    /// The manifest of a pulled image by its digest (how `envs.toml`
    /// references bases).
    pub fn manifest(&self, digest: &ImageDigest) -> Result<ImageManifest, ExternalImageError> {
        Ok(self.index()?.find_by_digest(digest)?.clone())
    }

    /// Resolve a manifest's layer digests into blob-CAS [`Layer`]s,
    /// newest (top) first.
    pub fn layers(&self, manifest: &ImageManifest) -> Result<Vec<Layer>, ExternalImageError> {
        Ok(self.blobs().resolve(&manifest.layers_top_first())?)
    }

    /// Record an already-materialized manifest without downloading
    /// (test support).
    #[cfg(test)]
    pub(crate) fn insert(&self, manifest: ImageManifest) -> Result<(), ExternalImageError> {
        let mut index = self.index()?;
        index.insert(manifest);
        index.save()?;
        Ok(())
    }
}
