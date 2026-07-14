//! [`Orca`]: the entry point representing one orca data root.
//!
//! The data root holds two independent areas — `envs/` (+ `run/`) and
//! `images/` — managed by sibling stores. `Orca` is the only place the
//! root path enters the library; everything else derives its paths from
//! here.

use std::path::PathBuf;

use crate::env::{EnvError, EnvStore};
use crate::external_image_store::ExternalImageStore;

/// One orca data root (`$HOME/.local/share/orca` by default; see
/// [`crate::orca_root`]).
pub struct Orca {
    root: PathBuf,
}

impl Orca {
    /// Bind to a data root. Cheap: nothing is read until an area store
    /// is opened.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The environment area (`envs/` and `run/`): loads `envs.toml`.
    pub fn envs(&self) -> Result<EnvStore, EnvError> {
        EnvStore::load(&self.root)
    }

    /// The external-image area (`images/`): pulled images and their
    /// blob CAS.
    pub fn external_images(&self) -> ExternalImageStore {
        ExternalImageStore::new(&self.root)
    }
}
