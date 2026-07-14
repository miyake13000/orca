//! [`ContainerImage`]: the material a container runs from (upper + lower stack +
//! base + config).
//!
//! Host-based and image-based environments are "the same thing with a
//! different bottom": both are represented by one struct, and the only
//! Host/Guest branching lives in consumers of [`Base`] (diff stacking here,
//! overlay mounting in `orca-container`'s `OverlayMount` impl — this crate
//! knows nothing about mounting).

use std::path::PathBuf;

use crate::layer::{Layer, Upper};

/// Static runtime defaults *declared by the image*.
///
/// This is pure image data: for pulled images it comes from the OCI config
/// (via the image index); host-based environments declare nothing, so their
/// config is [`ImageConfig::default`] (empty). The process environment is
/// never consulted here — resolving the actual argv / env / cwd for a run
/// is policy and lives in the `orca` crate (`ExecSpec`).
#[derive(Debug, Clone)]
pub struct ImageConfig {
    /// OCI entrypoint (empty when the image declares none).
    pub entrypoint: Vec<String>,
    /// OCI cmd (empty when the image declares none).
    pub cmd: Vec<String>,
    /// Environment variables as `KEY=VALUE` strings.
    pub env: Vec<String>,
    /// Declared working directory (`/` when unset).
    pub working_dir: PathBuf,
}

impl Default for ImageConfig {
    /// The empty declaration used for host-based environments.
    fn default() -> Self {
        Self {
            entrypoint: Vec::new(),
            cmd: Vec::new(),
            env: Vec::new(),
            working_dir: PathBuf::from("/"),
        }
    }
}

/// The base a [`ContainerImage`] sits on: the live host rootfs or the extracted
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
/// `ContainerImage` is pure material — it can be *stacked* (for diff) and *described*
/// (config), but mounting it is done via the `OverlayMount` trait that
/// `orca-container` implements for it.
#[derive(Debug, Clone)]
pub struct ContainerImage {
    /// The writable layer (`envs/<uuid>/diff/`).
    pub upper: Upper,
    /// Committed layers of the environment's history, newest first.
    pub lower: Vec<Layer>,
    /// The base under the committed layers.
    pub base: Base,
    /// Runtime defaults (argv / env / cwd material).
    pub config: ImageConfig,
}

impl ContainerImage {
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
