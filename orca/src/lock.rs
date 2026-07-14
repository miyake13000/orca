//! [`LockFile`]: the run/vcs shared exclusion mechanism.
//!
//! One lock file per environment (`run/<uuid>/lock`) containing the
//! holder's PID. `orca run` *acquires* it for the container's lifetime;
//! Image operations only *check* that no live lock exists. A lock
//! whose PID is dead is stale: `check` cleans it up (lock file plus the
//! leftover `session/` directory — mounts died with the child's namespace,
//! so directories are all that is left) and reports the slot free.

use std::path::{Path, PathBuf};

/// Errors from lock handling.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// A live container holds the lock.
    #[error("a container is already running in this environment (pid {0})")]
    Running(i32),
    /// Lock file I/O failed.
    #[error("lock file error: {0}")]
    Io(#[from] std::io::Error),
}

/// An acquired run lock. Released explicitly via [`release`](Self::release)
/// or best-effort on drop.
pub struct LockFile {
    path: PathBuf,
    released: bool,
}

impl LockFile {
    /// Acquire the lock for the calling process (used by `orca run`).
    ///
    /// Stale locks are cleaned up first; a live lock yields
    /// [`LockError::Running`]. Side effects: creates `run/<uuid>/` and
    /// writes this process's PID into the lock file.
    pub fn acquire(path: &Path) -> Result<Self, LockError> {
        if let Some(pid) = Self::check(path)? {
            return Err(LockError::Running(pid));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // create_new closes the acquire/acquire race window.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(file) => {
                use std::io::Write;
                let mut file = file;
                write!(file, "{}", std::process::id())?;
                Ok(Self {
                    path: path.to_path_buf(),
                    released: false,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(LockError::Running(0))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Check the lock without acquiring it (used by Image operations).
    ///
    /// Returns `Some(pid)` if a live process holds it, `None` if free.
    /// A stale lock (dead PID or unparsable content) is cleaned up along
    /// with the leftover `session/` directory before returning `None`.
    pub fn check(path: &Path) -> Result<Option<i32>, LockError> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if let Ok(pid) = content.trim().parse::<i32>()
            && pid > 0
            && pid_alive(pid)
        {
            return Ok(Some(pid));
        }
        // Stale: clean the lock and any leftover session directory.
        let _ = std::fs::remove_file(path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent.join("session"));
        }
        Ok(None)
    }

    /// Release the lock (removes the file).
    pub fn release(mut self) -> Result<(), LockError> {
        self.released = true;
        std::fs::remove_file(&self.path)?;
        Ok(())
    }
}

impl Drop for LockFile {
    /// Best-effort release so a panicking / erroring holder does not leave
    /// a live-looking lock behind. Never panics.
    fn drop(&mut self) {
        if !self.released {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Whether a process with `pid` exists (kill-0; EPERM counts as alive).
fn pid_alive(pid: i32) -> bool {
    let ret = unsafe { libc::kill(pid, 0) };
    if ret == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_conflict_and_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/uuid/lock");

        let lock = LockFile::acquire(&path).unwrap();
        // Our own PID is alive -> conflict.
        assert!(matches!(
            LockFile::acquire(&path),
            Err(LockError::Running(_))
        ));
        assert!(LockFile::check(&path).unwrap().is_some());
        lock.release().unwrap();
        assert!(LockFile::check(&path).unwrap().is_none());
    }

    #[test]
    fn stale_lock_is_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/uuid/lock");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A PID that cannot exist (> pid_max default).
        std::fs::write(&path, "999999999").unwrap();
        let session = path.parent().unwrap().join("session");
        std::fs::create_dir_all(session.join("rootfs")).unwrap();

        assert!(LockFile::check(&path).unwrap().is_none());
        assert!(!path.exists());
        assert!(!session.exists());

        // Garbage content is stale too.
        std::fs::write(&path, "not-a-pid").unwrap();
        assert!(LockFile::check(&path).unwrap().is_none());
    }

    #[test]
    fn drop_releases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock");
        {
            let _lock = LockFile::acquire(&path).unwrap();
            assert!(path.exists());
        }
        assert!(!path.exists());
    }
}
