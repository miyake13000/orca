//! [`OverlayMount`]: gives `orca_image::Image` the ability to mount itself.
//!
//! Overlay mounting happens inside the child's mount namespace, which is a
//! container concern — so the trait *and* its impl for `Image` live here
//! (orphan rule: the trait is local). `Image` itself stays mount-agnostic
//! in `orca-image`.

use std::path::PathBuf;

use orca_image::{Base, Image};

use crate::mount::{MountError, mount_fake_rootfs, overlay_mount};

/// Runtime paths under `run/<uuid>/session/`, prepared by the `orca` crate
/// and consumed by the container.
///
/// `base` is the session directory itself; `Container::run` creates it and
/// `Container::wait` (or failure cleanup) removes it wholesale. The
/// subdirectories are created on demand by [`OverlayMount::mount`].
#[derive(Debug, Clone)]
pub struct SessionPaths {
    /// The session directory (`run/<uuid>/session/`).
    pub base: PathBuf,
    /// Overlay mount point and `pivot_root` target (`session/rootfs`).
    pub rootfs: PathBuf,
    /// Overlay workdir (`session/work`).
    pub work: PathBuf,
    /// Host-based only: stage-1 overlay mount point (`session/fake_rootfs`).
    pub fake_rootfs: PathBuf,
    /// Host-based only: stage-1 throwaway upper (`session/fake_upper`).
    pub fake_upper: PathBuf,
    /// Host-based only: stage-1 workdir (`session/fake_work`).
    pub fake_work: PathBuf,
}

/// A mounted rootfs: the overlay mount point that `pivot_root` targets.
pub struct Rootfs(pub PathBuf);

/// The container-side capability of being overlay-mounted into a rootfs.
///
/// Contract: the caller must be inside the child's fresh mount namespace
/// and have called [`crate::mount::make_private`] beforehand.
pub trait OverlayMount {
    /// Mount the overlay stack and return the merged mount point.
    fn mount(&self, session: &SessionPaths) -> Result<Rootfs, MountError>;
}

impl OverlayMount for Image {
    /// Translate `base` into the bottom lower layers — the only place
    /// Host/Guest branch — then mount the main overlay:
    /// `lowerdir = committed layers (newest first) : base`, upper =
    /// `diff/`, workdir = `session/work`.
    fn mount(&self, session: &SessionPaths) -> Result<Rootfs, MountError> {
        let base_lowers: Vec<PathBuf> = match &self.base {
            // Host: `/` cannot sit in lowerdir directly (dentry-based
            // overlap check); fold it into fake_rootfs first (DESIGN §5).
            Base::Host => vec![mount_fake_rootfs(
                std::path::Path::new("/"),
                &session.fake_rootfs,
                &session.fake_upper,
                &session.fake_work,
            )?],
            Base::Guest(layers) => layers.iter().map(|l| l.path().to_path_buf()).collect(),
        };
        let mut lowerdir: Vec<PathBuf> =
            self.lower.iter().map(|l| l.path().to_path_buf()).collect();
        lowerdir.extend(base_lowers);
        overlay_mount(&session.rootfs, &lowerdir, self.upper.path(), &session.work)?;
        Ok(Rootfs(session.rootfs.clone()))
    }
}
