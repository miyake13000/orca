//! Layers, images and diff computation for orca.
//!
//! This crate owns everything about the *contents* of an environment:
//!
//! - [`Layer`] / [`Upper`] — directories holding OverlayFS layers, and their
//!   tree hashing ([`orca_hash::Hashable`] implementations).
//! - [`LayerStore`] — `envs/<uuid>/layers/` (committed layers).
//! - [`changes`] / [`Change`] — read-only diff of layer stacks, used by
//!   `orca diff` and `orca apply`.
//! - [`Image`] / [`Base`] / [`ImageConfig`] — the material a container runs
//!   from. Mounting it is *not* this crate's concern (the `OverlayMount`
//!   trait lives in `orca-container`).
//! - [`external_image`] — pulled OCI images: `images.toml` index, blob CAS
//!   and the registry downloader.
//!
//! On-disk layers are always stored in the OverlayFS native format:
//! whiteouts are 0:0 character devices and opaque directories carry the
//! `trusted.overlay.opaque` xattr. OCI tar whiteouts (`.wh.*`) are converted
//! at extraction time by the downloader.

#![warn(missing_docs)]

pub mod diff;
pub mod external_image;
mod image;
mod layer;
mod store;
pub(crate) mod whiteout;

pub use diff::{Blacklist, Change, DiffError, changes};
pub use image::{Base, Image, ImageConfig};
pub use layer::{Layer, Upper};
pub use store::{LayerStore, LayerStoreError};

use external_image::ImageDigest;
use serde::{Deserialize, Serialize};

/// Reference to the base an environment was created from.
///
/// Stored in `envs.toml` as `base = { type = "host" }` or
/// `base = { type = "external", image_digest = "<hex>" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BaseImageRef {
    /// The live host rootfs (`/`).
    Host,
    /// A pulled OCI image, identified by its manifest digest.
    External {
        /// Manifest digest keying into `images.toml`.
        image_digest: ImageDigest,
    },
}
