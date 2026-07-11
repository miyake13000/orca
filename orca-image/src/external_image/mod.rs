//! Pulled OCI images: reference parsing, image index, blob store
//! and registry downloader.
//!
//! Responsibilities are split three ways (DESIGN §8): tag resolution
//! ([`ImageIndex`]), resolved metadata records ([`ImageManifest`]), and the
//! content-addressed layer store ([`LayerBlobStore`]). Digest kinds are
//! separated by newtypes ([`ImageDigest`] vs [`LayerDigest`]) so they
//! cannot be mixed up.

mod blob_store;
mod downloader;

pub use blob_store::{BlobStoreError, LayerBlobStore};
pub use downloader::{Downloader, PullError};

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orca_hash::{Hash, hash_serde};
use serde::{Deserialize, Serialize};

use crate::image::ImageConfig;

/// Errors from the image index and reference handling.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// The requested image is not in the index.
    #[error("image not found: {0} (try `orca image pull {0}`)")]
    NotFound(String),
    /// No image with the given digest is in the index.
    #[error("image not found for digest {0}")]
    DigestNotFound(String),
    /// A digest string was not `sha256:<hex>` / `<hex>`.
    #[error("invalid digest: {0}")]
    InvalidDigest(String),
    /// The index file could not be read or written.
    #[error("failed to access image index: {0}")]
    Io(#[from] std::io::Error),
    /// The index file could not be parsed.
    #[error("failed to parse image index: {0}")]
    Parse(#[from] toml::de::Error),
    /// The index file could not be serialized.
    #[error("failed to serialize image index: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Digest of an image manifest (the index key). Always sha256.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ImageDigest(#[serde(with = "hash_serde")] pub Hash);

/// Digest of a layer blob (names `images/layers/sha256/<hex>/`). Always
/// sha256.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerDigest(#[serde(with = "hash_serde")] pub Hash);

impl fmt::Display for ImageDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", orca_hash::to_hex(&self.0))
    }
}

impl fmt::Display for LayerDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", orca_hash::to_hex(&self.0))
    }
}

/// Parse `sha256:<hex>` (or bare `<hex>`) into a raw [`Hash`].
pub(crate) fn parse_sha256(s: &str) -> Result<Hash, IndexError> {
    let hex = s.strip_prefix("sha256:").unwrap_or(s);
    orca_hash::from_hex(hex).map_err(|_| IndexError::InvalidDigest(s.to_string()))
}

/// A parsed image reference: `[registry/]repository[:tag]`.
///
/// Defaults: registry `docker.io`, tag `latest`; single-segment
/// repositories on docker.io get the `library/` prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Registry host, e.g. `docker.io` or `ghcr.io`.
    pub registry: String,
    /// Repository path, e.g. `library/ubuntu`.
    pub repository: String,
    /// Tag, e.g. `24.04`.
    pub tag: String,
}

impl Reference {
    /// Parse a reference string like `ubuntu:24.04`,
    /// `ghcr.io/owner/repo:v1` or `alpine`.
    pub fn parse(reference: &str) -> Self {
        // Split off the registry when the first path segment looks like a
        // host (contains '.' or ':' or is "localhost").
        let (registry, rest) = match reference.split_once('/') {
            Some((first, rest))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first.to_string(), rest.to_string())
            }
            _ => ("docker.io".to_string(), reference.to_string()),
        };
        // Tag separator: a ':' after the last '/'.
        let (repository, tag) = match rest.rsplit_once(':') {
            Some((repo, tag)) if !tag.contains('/') => (repo.to_string(), tag.to_string()),
            _ => (rest, "latest".to_string()),
        };
        let repository = if registry == "docker.io" && !repository.contains('/') {
            format!("library/{repository}")
        } else {
            repository
        };
        Self {
            registry,
            repository,
            tag,
        }
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}:{}", self.registry, self.repository, self.tag)
    }
}

/// One index record: fully resolved metadata of a pulled image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageManifest {
    /// Manifest digest (identity of the pulled image).
    pub digest: ImageDigest,
    /// Registry host the image was pulled from (e.g. `docker.io`).
    pub registry: String,
    /// Repository path (e.g. `library/ubuntu`).
    pub repository: String,
    /// Tag the image was pulled as.
    pub tag: String,
    /// Layer blob digests, in manifest order (oldest/bottom first).
    #[serde(rename = "layers")]
    pub layer_digests: Vec<LayerDigest>,
    /// OCI entrypoint.
    pub entrypoint: Vec<String>,
    /// OCI cmd.
    pub cmd: Vec<String>,
    /// Image environment (`KEY=VALUE`).
    pub env: Vec<String>,
    /// Image working directory.
    pub working_dir: PathBuf,
    /// When the image was pulled.
    pub pulled_at: DateTime<Utc>,
}

impl ImageManifest {
    /// Convert the image's runtime defaults into an [`ImageConfig`].
    pub fn config(&self) -> ImageConfig {
        ImageConfig {
            entrypoint: self.entrypoint.clone(),
            cmd: self.cmd.clone(),
            env: self.env.clone(),
            working_dir: if self.working_dir.as_os_str().is_empty() {
                PathBuf::from("/")
            } else {
                self.working_dir.clone()
            },
        }
    }

    /// Layer digests ordered newest (top) first, the order overlay lower
    /// stacks and diff stacks use.
    pub fn layers_top_first(&self) -> Vec<LayerDigest> {
        let mut layers = self.layer_digests.clone();
        layers.reverse();
        layers
    }
}

/// On-disk shape of the index file.
#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexFile {
    #[serde(default)]
    images: Vec<ImageManifest>,
}

/// The image index: resolves references and digests to
/// [`ImageManifest`] records, backed by a TOML file at a caller-supplied
/// path.
pub struct ImageIndex {
    path: PathBuf,
    images: Vec<ImageManifest>,
}

impl ImageIndex {
    /// Load the index from the file at `file_path` (missing file = empty
    /// index).
    pub fn load(file_path: &Path) -> Result<Self, IndexError> {
        let path = file_path.to_path_buf();
        let images = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<IndexFile>(&text)?.images,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, images })
    }

    /// Resolve a reference string (`ubuntu:24.04`, `ghcr.io/x/y:v1`) to a
    /// pulled image.
    pub fn resolve(&self, reference: &str) -> Result<&ImageManifest, IndexError> {
        let r = Reference::parse(reference);
        self.images
            .iter()
            .find(|m| m.registry == r.registry && m.repository == r.repository && m.tag == r.tag)
            .ok_or_else(|| IndexError::NotFound(reference.to_string()))
    }

    /// Find a pulled image by manifest digest.
    pub fn find_by_digest(&self, digest: &ImageDigest) -> Result<&ImageManifest, IndexError> {
        self.images
            .iter()
            .find(|m| &m.digest == digest)
            .ok_or_else(|| IndexError::DigestNotFound(digest.to_string()))
    }

    /// Insert (or replace, keyed by registry/repository/tag) a manifest.
    pub fn insert(&mut self, manifest: ImageManifest) {
        self.images.retain(|m| {
            !(m.registry == manifest.registry
                && m.repository == manifest.repository
                && m.tag == manifest.tag)
        });
        self.images.push(manifest);
    }

    /// Remove the record for `reference`.
    pub fn remove(&mut self, reference: &str) -> Result<ImageManifest, IndexError> {
        let r = Reference::parse(reference);
        let pos = self
            .images
            .iter()
            .position(|m| m.registry == r.registry && m.repository == r.repository && m.tag == r.tag)
            .ok_or_else(|| IndexError::NotFound(reference.to_string()))?;
        Ok(self.images.remove(pos))
    }

    /// All records.
    pub fn list(&self) -> Vec<&ImageManifest> {
        self.images.iter().collect()
    }

    /// The set of layer digests referenced by any image — the gc roots for
    /// [`LayerBlobStore::gc`] (layers are shared between images, so
    /// deletion is reference-set based).
    pub fn referenced_layers(&self) -> HashSet<LayerDigest> {
        self.images
            .iter()
            .flat_map(|m| m.layer_digests.iter().copied())
            .collect()
    }

    /// Persist the index, creating parent directories.
    pub fn save(&self) -> Result<(), IndexError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = IndexFile {
            images: self.images.clone(),
        };
        std::fs::write(&self.path, toml::to_string_pretty(&file)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_parsing() {
        assert_eq!(
            Reference::parse("ubuntu:24.04"),
            Reference {
                registry: "docker.io".into(),
                repository: "library/ubuntu".into(),
                tag: "24.04".into(),
            }
        );
        assert_eq!(
            Reference::parse("alpine"),
            Reference {
                registry: "docker.io".into(),
                repository: "library/alpine".into(),
                tag: "latest".into(),
            }
        );
        assert_eq!(
            Reference::parse("ghcr.io/owner/repo:v1"),
            Reference {
                registry: "ghcr.io".into(),
                repository: "owner/repo".into(),
                tag: "v1".into(),
            }
        );
        assert_eq!(
            Reference::parse("localhost:5000/foo"),
            Reference {
                registry: "localhost:5000".into(),
                repository: "foo".into(),
                tag: "latest".into(),
            }
        );
    }

    fn manifest(tag: &str, digest: u8) -> ImageManifest {
        ImageManifest {
            digest: ImageDigest([digest; 32]),
            registry: "docker.io".into(),
            repository: "library/ubuntu".into(),
            tag: tag.into(),
            layer_digests: vec![LayerDigest([digest; 32])],
            entrypoint: vec![],
            cmd: vec!["/bin/bash".into()],
            env: vec![],
            working_dir: "/".into(),
            pulled_at: Utc::now(),
        }
    }

    #[test]
    fn index_roundtrip_and_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("images.toml");
        let mut index = ImageIndex::load(&file).unwrap();
        index.insert(manifest("24.04", 1));
        index.insert(manifest("25.04", 2));
        index.save().unwrap();

        let index = ImageIndex::load(&file).unwrap();
        assert_eq!(index.list().len(), 2);
        let m = index.resolve("ubuntu:24.04").unwrap();
        assert_eq!(m.digest, ImageDigest([1; 32]));
        assert!(index.resolve("ubuntu:9.99").is_err());
        assert!(index.find_by_digest(&ImageDigest([2; 32])).is_ok());
        assert_eq!(index.referenced_layers().len(), 2);
    }

    #[test]
    fn insert_replaces_same_tag() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = ImageIndex::load(&dir.path().join("images.toml")).unwrap();
        index.insert(manifest("24.04", 1));
        index.insert(manifest("24.04", 3));
        assert_eq!(index.list().len(), 1);
        assert_eq!(
            index.resolve("ubuntu:24.04").unwrap().digest,
            ImageDigest([3; 32])
        );
    }

    #[test]
    fn layers_top_first_reverses_manifest_order() {
        let mut m = manifest("24.04", 1);
        m.layer_digests = vec![LayerDigest([1; 32]), LayerDigest([2; 32])];
        assert_eq!(
            m.layers_top_first(),
            vec![LayerDigest([2; 32]), LayerDigest([1; 32])]
        );
    }
}
