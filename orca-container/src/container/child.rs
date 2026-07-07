//! The child process: overlay mount, `pivot_root`, pseudo filesystems,
//! PTY setup, and `exec`.
//!
//! Everything here runs after `clone(CLONE_NEWNS | ...)` in the child.
//! There is no rollback code: all mounts live in the child's namespace and
//! evaporate on `_exit`, so failures just report a stage over the CLOEXEC
//! status pipe and `_exit(1)`. `std::process::exit` is never used (no
//! atexit / stdio double-flush after the fork-like clone).
//!
//! Init steps are split into *required* (overlay, pivot_root, /proc, /dev,
//! devpts, PTY in tty mode — failure aborts) and *best-effort* (/sys,
//! /dev/shm, /dev/mqueue, cgroup, device nodes, symlinks, resolv.conf,
//! /dev/console — failure prints one warning and continues).

use std::ffi::CString;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use nix::mount::MsFlags;
use nix::sys::stat::{Mode, SFlag, makedev, mknod};
use orca_image::Image;

use crate::image::{OverlayMount, SessionPaths};
use crate::mount::{self, PseudoMount};

use super::tty::send_fd;
use super::{IoMode, RunAs};

/// Everything the child needs, borrowed from the parent's (copied)
/// address space.
pub(crate) struct ChildConfig<'a> {
    pub image: &'a Image,
    pub session: &'a SessionPaths,
    pub io: IoMode,
    pub cmd: &'a [String],
    pub env: &'a [String],
    pub cwd: &'a Path,
    pub run_as: Option<&'a RunAs>,
    pub hostname: Option<&'a str>,
    pub set_hostname: bool,
    /// Write end of the CLOEXEC status pipe.
    pub status_fd: RawFd,
    /// Child end of the fd-passing socketpair (tty mode).
    pub sock_fd: RawFd,
}

/// Report a failed stage over the status pipe and `_exit(1)`. Never
/// returns.
fn fail(status_fd: RawFd, stage: &str, detail: String) -> ! {
    let msg = format!("{stage}: {detail}");
    // Best-effort write; there is nothing to do about failure here.
    unsafe {
        libc::write(
            status_fd,
            msg.as_ptr() as *const libc::c_void,
            msg.len(),
        );
        libc::_exit(1);
    }
}

/// Print a one-line warning for a failed best-effort step.
fn warn(step: &str, err: impl std::fmt::Display) {
    eprintln!("orca: warning: {step}: {err}");
}

/// Entry point of the cloned child. Ends in `exec` on success or
/// `_exit(1)` after reporting; the `isize` return type only satisfies the
/// clone callback signature.
pub(crate) fn child_main(cfg: &ChildConfig<'_>) -> isize {
    let status = cfg.status_fd;

    // 1. Stop mount propagation to the host.
    if let Err(e) = mount::make_private() {
        fail(status, "make_private", e.to_string());
    }

    // 2. Mount the overlay stack (fake_rootfs two-step for host bases).
    let rootfs = match cfg.image.mount(cfg.session) {
        Ok(r) => r,
        Err(e) => fail(status, "overlay_mount", e.to_string()),
    };

    // 3. Hostname (UTS namespace only).
    if cfg.set_hostname {
        let name = cfg.hostname.unwrap_or("orca");
        if let Err(e) = nix::unistd::sethostname(name) {
            warn("sethostname", e);
        }
    }

    // 4. pivot_root into the overlay.
    let old_root = rootfs.0.join("oldroot");
    if let Err(e) = std::fs::create_dir_all(&old_root) {
        fail(status, "pivot_root(mkdir oldroot)", e.to_string());
    }
    if let Err(e) = nix::unistd::pivot_root(&rootfs.0, &old_root) {
        fail(status, "pivot_root", e.to_string());
    }
    if let Err(e) = nix::unistd::chdir("/") {
        fail(status, "chdir(/)", e.to_string());
    }

    // 5. Pseudo filesystems (required ones first).
    for m in REQUIRED_MOUNTS {
        if let Err(e) = m.mount() {
            fail(status, m.target, e.to_string());
        }
    }
    for m in BEST_EFFORT_MOUNTS {
        if let Err(e) = m.mount() {
            warn(m.target, e);
        }
    }

    // 6. Device nodes and standard symlinks (best-effort).
    populate_dev();

    // 7. resolv.conf from the old root (best-effort; skip if absent).
    copy_resolv_conf();

    // 8. PTY setup (tty mode): allocate in our own devpts, wire stdio,
    //    ship the master to the parent.
    if cfg.io == IoMode::Tty
        && let Err(e) = setup_tty(cfg.sock_fd)
    {
        fail(status, "pty", e);
    }

    // 9. Drop the old root; only resolv.conf depended on it.
    if let Err(e) = mount::unmount_old_root(Path::new("/oldroot")) {
        fail(status, "umount(/oldroot)", e.to_string());
    }

    // 10. Complete privilege drop (required when requested; uid last so
    //     the preceding group changes are still permitted).
    if let Some(run_as) = cfg.run_as
        && let Err(e) = drop_privileges(run_as)
    {
        fail(status, "drop_privileges", e);
    }

    // 11. Working directory (checked as the final identity; fall back
    //     to /).
    if nix::unistd::chdir(cfg.cwd).is_err() {
        let _ = nix::unistd::chdir("/");
    }

    // 12. exec. Reaching it closes the CLOEXEC status pipe -> parent EOF.
    let argv: Vec<CString> = cfg
        .cmd
        .iter()
        .map(|s| CString::new(s.as_str()).unwrap_or_default())
        .collect();
    let envp: Vec<CString> = cfg
        .env
        .iter()
        .map(|s| CString::new(s.as_str()).unwrap_or_default())
        .collect();
    let program = argv.first().cloned().unwrap_or_default();
    let err = nix::unistd::execvpe(&program, &argv, &envp).unwrap_err();
    fail(
        status,
        "exec",
        format!("{}: {err}", program.to_string_lossy()),
    );
}

/// Required pseudo mounts (failure aborts container start).
const REQUIRED_MOUNTS: &[PseudoMount] = &[
    PseudoMount {
        target: "/proc",
        fstype: "proc",
        source: "proc",
        flags: MsFlags::empty(),
        data: None,
    },
    PseudoMount {
        target: "/dev",
        fstype: "tmpfs",
        source: "tmpfs",
        flags: MsFlags::MS_NOSUID.union(MsFlags::MS_STRICTATIME),
        data: Some("mode=755,size=65536k"),
    },
    PseudoMount {
        target: "/dev/pts",
        fstype: "devpts",
        source: "devpts",
        flags: MsFlags::MS_NOSUID.union(MsFlags::MS_NOEXEC),
        // gid=5 = the conventional "tty" group (OCI default), so slaves
        // are rw for the owner and w for group tty (mode=0620).
        data: Some("newinstance,ptmxmode=0666,mode=0620,gid=5"),
    },
];

/// Best-effort pseudo mounts (failure logs a warning).
const BEST_EFFORT_MOUNTS: &[PseudoMount] = &[
    PseudoMount {
        target: "/sys",
        fstype: "sysfs",
        source: "sysfs",
        flags: MsFlags::MS_NOSUID
            .union(MsFlags::MS_NOEXEC)
            .union(MsFlags::MS_NODEV)
            .union(MsFlags::MS_RDONLY),
        data: None,
    },
    PseudoMount {
        target: "/dev/shm",
        fstype: "tmpfs",
        source: "shm",
        flags: MsFlags::MS_NOSUID
            .union(MsFlags::MS_NOEXEC)
            .union(MsFlags::MS_NODEV),
        data: Some("mode=1777,size=65536k"),
    },
    PseudoMount {
        target: "/dev/mqueue",
        fstype: "mqueue",
        source: "mqueue",
        flags: MsFlags::MS_NOSUID
            .union(MsFlags::MS_NOEXEC)
            .union(MsFlags::MS_NODEV),
        data: None,
    },
    PseudoMount {
        target: "/sys/fs/cgroup",
        fstype: "cgroup2",
        source: "cgroup2",
        flags: MsFlags::MS_NOSUID
            .union(MsFlags::MS_NOEXEC)
            .union(MsFlags::MS_NODEV)
            .union(MsFlags::MS_RELATIME)
            .union(MsFlags::MS_RDONLY),
        data: None,
    },
];

/// Create the standard device nodes (mknod; we run as real root, so no
/// bind-mount fallback is needed — DESIGN §5) and symlinks in the fresh
/// /dev tmpfs. All best-effort.
fn populate_dev() {
    const DEVICES: &[(&str, u64, u64)] = &[
        ("/dev/null", 1, 3),
        ("/dev/zero", 1, 5),
        ("/dev/full", 1, 7),
        ("/dev/random", 1, 8),
        ("/dev/urandom", 1, 9),
        ("/dev/tty", 5, 0),
    ];
    for (path, major, minor) in DEVICES {
        let mode = Mode::from_bits_truncate(0o666);
        if let Err(e) = mknod(*path, SFlag::S_IFCHR, mode, makedev(*major, *minor)) {
            warn(path, e);
            continue;
        }
        // mknod(2) masks the mode with the caller's umask (022 turns 666
        // into 644, breaking e.g. `su` + /dev/null); the device table is
        // authoritative, so enforce the mode explicitly (chmod ignores
        // the umask).
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))
        {
            warn(path, e);
        }
    }
    const SYMLINKS: &[(&str, &str)] = &[
        ("pts/ptmx", "/dev/ptmx"),
        ("/proc/self/fd", "/dev/fd"),
        ("/proc/self/fd/0", "/dev/stdin"),
        ("/proc/self/fd/1", "/dev/stdout"),
        ("/proc/self/fd/2", "/dev/stderr"),
    ];
    for (target, link) in SYMLINKS {
        if let Err(e) = std::os::unix::fs::symlink(target, link) {
            warn(link, e);
        }
    }
}

/// Become `run_as` completely: supplementary groups, then gid, then uid,
/// each including the saved id (`setresgid` / `setresuid` with all three
/// equal). Dropping the saved id too means the process cannot regain root
/// and — equally important — shells see euid == ruid and start normally
/// instead of entering their setuid security mode (which suppresses rc
/// files).
fn drop_privileges(run_as: &RunAs) -> Result<(), String> {
    use nix::unistd::{Gid, Uid, setgroups, setresgid, setresuid};

    let groups: Vec<Gid> = run_as.groups.iter().map(|g| Gid::from_raw(*g)).collect();
    setgroups(&groups).map_err(|e| format!("setgroups: {e}"))?;
    let gid = Gid::from_raw(run_as.gid);
    setresgid(gid, gid, gid).map_err(|e| format!("setresgid({gid}): {e}"))?;
    let uid = Uid::from_raw(run_as.uid);
    setresuid(uid, uid, uid).map_err(|e| format!("setresuid({uid}): {e}"))?;
    Ok(())
}

/// Copy the host's resolv.conf (via /oldroot) so DNS works even when the
/// container rootfs has none or a dangling systemd-resolved symlink.
/// Best-effort: absent source is silently skipped.
fn copy_resolv_conf() {
    let src = Path::new("/oldroot/etc/resolv.conf");
    let Ok(content) = std::fs::read(src) else {
        return; // absent or unreadable -> skip
    };
    let dst = Path::new("/etc/resolv.conf");
    // Already identical (e.g. host resolv.conf is a regular file visible
    // through the overlay): skip, so a plain `orca run` does not dirty
    // the upper.
    if std::fs::read(dst).is_ok_and(|existing| existing == content) {
        return;
    }
    // Remove a pre-existing file/symlink so we do not write through a
    // (possibly dangling) symlink into /run.
    let _ = std::fs::remove_file(dst);
    if let Err(e) = std::fs::write(dst, content) {
        warn("resolv.conf", e);
    }
}

/// Allocate a PTY from our own devpts, make its slave our controlling
/// stdio, and ship the master fd to the parent over `SCM_RIGHTS`.
///
/// Errors are returned as strings for the status-pipe report (required
/// step: failure aborts the container in tty mode).
fn setup_tty(sock_fd: RawFd) -> Result<(), String> {
    use nix::fcntl::OFlag;
    use nix::pty::{grantpt, posix_openpt, ptsname, unlockpt};

    let master =
        posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY).map_err(|e| format!("openpt: {e}"))?;
    grantpt(&master).map_err(|e| format!("grantpt: {e}"))?;
    unlockpt(&master).map_err(|e| format!("unlockpt: {e}"))?;
    // SAFETY: single-threaded child; ptsname's static buffer is unshared.
    let slave_path = unsafe { ptsname(&master) }.map_err(|e| format!("ptsname: {e}"))?;

    nix::unistd::setsid().map_err(|e| format!("setsid: {e}"))?;
    // Opening the slave without O_NOCTTY makes it our controlling tty
    // (we are a session leader with no controlling terminal yet).
    let slave = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PathBuf::from(&slave_path))
        .map_err(|e| format!("open {slave_path}: {e}"))?;
    let slave_fd = slave.as_raw_fd();
    for target in 0..=2 {
        nix::unistd::dup2(slave_fd, target).map_err(|e| format!("dup2({target}): {e}"))?;
    }

    // Best-effort: expose the slave as /dev/console (OCI-style bind).
    let console = Path::new("/dev/console");
    if std::fs::File::create(console).is_ok()
        && let Err(e) = mount::bind_mount(Path::new(&slave_path), console)
    {
        warn("/dev/console", e);
    }

    // Hand the master to the parent, then drop our copy.
    send_fd(sock_fd, master.as_raw_fd()).map_err(|e| format!("send master: {e}"))?;
    drop(master);
    drop(slave); // 0/1/2 keep the tty open
    let _ = sock_fd; // parent's recv unblocks; socket closes at exec
    Ok(())
}
