//! [`Image`]: the material a container runs from (upper + lower stack +
//! base + config).
//!
//! Host-based and image-based environments are "the same thing with a
//! different bottom": both are represented by one struct, and the only
//! Host/Guest branching lives in consumers of [`Base`] (diff stacking here,
//! overlay mounting in `orca-container`'s `OverlayMount` impl — this crate
//! knows nothing about mounting).

use std::path::PathBuf;

use crate::layer::{Layer, Upper};

/// Runtime defaults derived from the image (or host) used to resolve the
/// container's argv / env / working directory.
#[derive(Debug, Clone)]
pub struct ImageConfig {
    /// OCI entrypoint (empty for host-based environments).
    pub entrypoint: Vec<String>,
    /// OCI cmd (`["/bin/bash"]` for host-based environments).
    pub cmd: Vec<String>,
    /// Environment variables as `KEY=VALUE` strings.
    pub env: Vec<String>,
    /// Initial working directory (falls back to `/` if missing in the
    /// container).
    pub working_dir: PathBuf,
}

impl ImageConfig {
    /// Fallback PATH used when the host has no PATH set.
    const DEFAULT_PATH: &'static str =
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

    /// Defaults for host-based environments: no entrypoint,
    /// `cmd=["/bin/bash"]`, env = host PATH (+ TERM if set), cwd = `/`.
    pub fn host_default() -> Self {
        let mut env = vec![
            std::env::var("PATH")
                .map(|p| format!("PATH={p}"))
                .unwrap_or_else(|_| Self::DEFAULT_PATH.to_string()),
        ];
        if let Ok(term) = std::env::var("TERM") {
            env.push(format!("TERM={term}"));
        }
        Self {
            entrypoint: Vec::new(),
            cmd: vec!["/bin/bash".to_string()],
            env,
            working_dir: PathBuf::from("/"),
        }
    }
}

/// The base an [`Image`] sits on: the live host rootfs or the extracted
/// layers of a pulled image.
#[derive(Debug, Clone)]
pub enum Base {
    /// Host rootfs. As a diff baseline this is `/`; how it is stacked into
    /// an overlay (the fake_rootfs two-step) is `orca-container`'s concern.
    Host,
    /// Extracted image layers, newest first.
    Guest(Vec<Layer>),
}

/// Everything needed to run or diff an environment: the writable upper,
/// the committed lower stack, the base, and the runtime config.
///
/// `Image` is pure material — it can be *stacked* (for diff) and *described*
/// (config), but mounting it is done via the `OverlayMount` trait that
/// `orca-container` implements for it.
#[derive(Debug, Clone)]
pub struct Image {
    /// The writable layer (`envs/<uuid>/diff/`).
    pub upper: Upper,
    /// Committed layers of the environment's history, newest first.
    pub lower: Vec<Layer>,
    /// The base under the committed layers.
    pub base: Base,
    /// Runtime defaults (argv / env / cwd material).
    pub config: ImageConfig,
}

impl Image {
    /// The stack `orca diff` compares the upper against: committed layers
    /// plus the base (host `/` or image layers), top to bottom.
    pub fn baseline(&self) -> Vec<Layer> {
        let mut stack = self.lower.clone();
        stack.extend(self.base_only());
        stack
    }

    /// Only the base as a stack (the `base` side of `orca apply`); the
    /// host base becomes a single layer at `/`.
    pub fn base_only(&self) -> Vec<Layer> {
        match &self.base {
            Base::Host => vec![Layer::new(PathBuf::from("/"))],
            Base::Guest(layers) => layers.clone(),
        }
    }

    /// The runtime configuration.
    pub fn config(&self) -> &ImageConfig {
        &self.config
    }
}
