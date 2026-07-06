//! Mount helpers used inside the child's mount namespace.
//!
//! All functions here mutate mount state and must only be called in the
//! child process after `clone(CLONE_NEWNS)`; the mounts vanish with the
//! namespace when the child exits, so no unmount bookkeeping is needed on
//! the host side.

use std::path::{Path, PathBuf};

use nix::mount::{MntFlags, MsFlags, mount, umount2};

/// Errors from mount operations, carrying the target path for context.
#[derive(Debug, thiserror::Error)]
pub enum MountError {
    /// A mount / umount syscall failed.
    #[error("mount operation on {path} failed: {source}")]
    Syscall {
        /// Mount target.
        path: PathBuf,
        /// errno.
        source: nix::Error,
    },
    /// Creating a mount point directory failed.
    #[error("failed to create mount point {path}: {source}")]
    Mkdir {
        /// Directory being created.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
}

fn syscall_err(path: &Path, source: nix::Error) -> MountError {
    MountError::Syscall {
        path: path.to_path_buf(),
        source,
    }
}

fn mkdir_all(path: &Path) -> Result<(), MountError> {
    std::fs::create_dir_all(path).map_err(|source| MountError::Mkdir {
        path: path.to_path_buf(),
        source,
    })
}

/// Make the whole mount tree private (`/` gets `MS_REC | MS_PRIVATE`) so
/// nothing the child mounts propagates back to the host.
///
/// Precondition: called once in the child, before any other mount here.
pub fn make_private() -> Result<(), MountError> {
    mount(
        None::<&str>,
        "/",
        None::<&str>,
        MsFlags::MS_REC | MsFlags::MS_PRIVATE,
        None::<&str>,
    )
    .map_err(|e| syscall_err(Path::new("/"), e))
}

/// Mount an overlay at `target` with the given lower stack (highest
/// priority first), upper and workdir. Creates `target` and `work` as
/// needed; `upper` must already exist (it is the persistent `diff/`).
///
/// Note: overlay option strings cannot escape `:` or `,`; orca's layer
/// paths never contain them (uuid/hex names under the orca root).
pub fn overlay_mount(
    target: &Path,
    lowers: &[PathBuf],
    upper: &Path,
    work: &Path,
) -> Result<(), MountError> {
    mkdir_all(target)?;
    mkdir_all(work)?;
    let lowerdir = lowers
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":");
    let data = format!(
        "lowerdir={},upperdir={},workdir={}",
        lowerdir,
        upper.display(),
        work.display()
    );
    mount(
        Some("overlay"),
        target,
        Some("overlay"),
        MsFlags::empty(),
        Some(data.as_str()),
    )
    .map_err(|e| syscall_err(target, e))
}

/// Stage-1 mount for host-based environments: overlay `src` (the host `/`)
/// to `mp` using a throwaway upper/workdir, returning `mp`.
///
/// The intermediate overlay exists purely to mint a rootfs view whose
/// dentries have no ancestry relation with the orca storage directories:
/// putting `/` directly in `lowerdir` (or bind-mounting it) trips the
/// kernel's dentry-based overlapping-layer check against `diff/` and the
/// layer dirs (see DESIGN §5). The upper never receives writes — stage 2
/// only reads this mount as a lower — and is destroyed with the session.
pub fn mount_fake_rootfs(
    src: &Path,
    mp: &Path,
    fake_upper: &Path,
    fake_work: &Path,
) -> Result<PathBuf, MountError> {
    mkdir_all(fake_upper)?;
    overlay_mount(mp, &[src.to_path_buf()], fake_upper, fake_work)?;
    Ok(mp.to_path_buf())
}

/// Detach the old root after `pivot_root` (`MNT_DETACH`) and remove the
/// mount point directory (best-effort).
pub fn unmount_old_root(old_root: &Path) -> Result<(), MountError> {
    umount2(old_root, MntFlags::MNT_DETACH).map_err(|e| syscall_err(old_root, e))?;
    let _ = std::fs::remove_dir(old_root);
    Ok(())
}

/// A pseudo-filesystem mount inside the container (OCI-style table entry).
pub(crate) struct PseudoMount {
    /// Mount target inside the container (absolute, post-pivot).
    pub target: &'static str,
    /// Filesystem type (`proc`, `sysfs`, `tmpfs`, ...).
    pub fstype: &'static str,
    /// Source name (conventionally the fstype or a label).
    pub source: &'static str,
    /// Mount flags.
    pub flags: MsFlags,
    /// Filesystem-specific data string.
    pub data: Option<&'static str>,
}

impl PseudoMount {
    /// Perform the mount, creating the target directory.
    pub fn mount(&self) -> Result<(), MountError> {
        let target = Path::new(self.target);
        mkdir_all(target)?;
        mount(
            Some(self.source),
            target,
            Some(self.fstype),
            self.flags,
            self.data,
        )
        .map_err(|e| syscall_err(target, e))
    }
}

/// Bind-mount `src` onto `dst` (used for `/dev/console`).
pub(crate) fn bind_mount(src: &Path, dst: &Path) -> Result<(), MountError> {
    mount(
        Some(src),
        dst,
        None::<&str>,
        MsFlags::MS_BIND,
        None::<&str>,
    )
    .map_err(|e| syscall_err(dst, e))
}
