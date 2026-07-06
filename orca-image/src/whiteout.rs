//! OverlayFS whiteout helpers (native on-disk format).
//!
//! orca stores every layer in the OverlayFS native format: a deletion is a
//! 0:0 character device ("whiteout") and an opaque directory carries the
//! `trusted.overlay.opaque = "y"` xattr. The downloader converts OCI tar
//! whiteouts (`.wh.*`) into this format at extraction time, so diff
//! computation only ever needs to understand the native format.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

/// Name of the opaque-directory xattr (requires CAP_SYS_ADMIN to write).
pub(crate) const OPAQUE_XATTR: &str = "trusted.overlay.opaque";

/// Whether the (symlink-)metadata describes an OverlayFS whiteout:
/// a character device with device number 0:0.
pub(crate) fn is_whiteout(md: &std::fs::Metadata) -> bool {
    md.file_type().is_char_device() && md.rdev() == 0
}

/// Whether `path` is an opaque directory (`trusted.overlay.opaque = "y"`).
///
/// Read errors (including unsupported xattrs) are treated as "not opaque".
pub(crate) fn is_opaque(path: &Path) -> bool {
    xattr::get(path, OPAQUE_XATTR)
        .ok()
        .flatten()
        .is_some_and(|v| v == b"y")
}

/// Create a whiteout (0:0 char device) at `path`.
///
/// Requires CAP_MKNOD (orca runs as root). Used by the downloader when
/// converting OCI tar whiteouts.
pub(crate) fn make_whiteout(path: &Path) -> std::io::Result<()> {
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    // Whiteouts are conventionally mode 0; overlayfs only looks at type+rdev.
    let ret = unsafe { libc::mknod(cpath.as_ptr(), libc::S_IFCHR, 0) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Mark the directory at `path` opaque (`trusted.overlay.opaque = "y"`).
///
/// Requires CAP_SYS_ADMIN (trusted.* namespace). Used by the downloader
/// when converting `.wh..wh..opq` entries.
pub(crate) fn set_opaque(path: &Path) -> std::io::Result<()> {
    xattr::set(path, OPAQUE_XATTR, b"y")
}
