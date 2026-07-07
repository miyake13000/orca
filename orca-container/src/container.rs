//! [`Container`] typestate, [`ContainerBuilder`] and process orchestration.
//!
//! Lifecycle: `ContainerBuilder::build()` (no side effects) →
//! `Container<Created>::run()` (session dir, `clone`, init handshake) →
//! `Container<Running>::wait()` (waitpid, teardown).
//!
//! Synchronization with the child uses exactly two one-way channels
//! (DESIGN §5):
//! - a CLOEXEC status pipe: EOF = the child reached `exec` (success),
//!   data = which init stage failed;
//! - a Unix socketpair carrying the PTY master fd via `SCM_RIGHTS`
//!   (tty mode only). The parent never `setns`es into the child.

use std::os::fd::AsRawFd;
use std::path::PathBuf;

use nix::sched::CloneFlags;
use nix::unistd::Pid;
use orca_image::Image;

use crate::image::SessionPaths;
use crate::mount::MountError;
use crate::{STACK_SIZE, container::tty::TtyConnector};

pub(crate) mod child;
pub(crate) mod parent;
pub(crate) mod tty;

/// Errors from building, running or waiting on a container.
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    /// Creating or removing the session directory failed.
    #[error("session directory {path}: {source}")]
    Session {
        /// The session path.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// A syscall in the parent failed (pipe/socketpair/clone/waitpid...).
    #[error("{op} failed: {source}")]
    Syscall {
        /// Which operation failed.
        op: &'static str,
        /// errno.
        source: nix::Error,
    },
    /// The child reported an initialization failure before `exec`.
    #[error("container initialization failed at {0}")]
    ChildInit(String),
    /// The child never completed the init handshake in time.
    #[error("timed out waiting for container initialization")]
    InitTimeout,
    /// Mount error surfaced through the child report path.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// The command has an empty argv (no user command and no image cmd).
    #[error("no command to execute (empty argv)")]
    EmptyCommand,
    /// Terminal / PTY handling failed in the parent.
    #[error("terminal error: {0}")]
    Tty(String),
}

pub(crate) fn syscall(op: &'static str) -> impl FnOnce(nix::Error) -> ContainerError {
    move |source| ContainerError::Syscall { op, source }
}

/// Which namespaces to unshare (mount is always unshared).
#[derive(Debug, Clone, Copy)]
pub struct NamespaceOpts {
    /// New PID namespace (child becomes PID 1). Default: true.
    pub pid: bool,
    /// New UTS namespace (container hostname). Default: true.
    pub uts: bool,
    /// New IPC namespace. Default: true.
    pub ipc: bool,
    /// New network namespace (isolated, no interfaces). Default: false —
    /// the network is shared with the host.
    pub net: bool,
}

impl Default for NamespaceOpts {
    fn default() -> Self {
        Self {
            pid: true,
            uts: true,
            ipc: true,
            net: false,
        }
    }
}

/// How the container's stdio is wired. The CLI decides once via
/// `isatty(stdin) && isatty(stdout)` and passes the result in — every
/// terminal syscall in this crate is downstream of that decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    /// Allocate a PTY in the container, receive the master over
    /// `SCM_RIGHTS`, put the host tty in raw mode and relay.
    Tty,
    /// No PTY: the child inherits the parent's fds 0/1/2 unchanged. The
    /// parent forwards SIGINT/SIGTERM/SIGHUP to the child instead.
    Piped,
}

/// The identity the container process drops to right before `exec`.
///
/// A resolved input on par with [`NamespaceOpts`]: deciding *who* to run
/// as is policy (the `orca` crate's `ExecSpec`); this crate merely
/// executes `setgroups(groups)` → `setresgid(gid)` → `setresuid(uid)` in
/// the child (uid last, saved ids included, so the drop is complete and
/// shells do not enter their setuid security mode).
#[derive(Debug, Clone)]
pub struct RunAs {
    /// Real/effective/saved uid to become.
    pub uid: u32,
    /// Real/effective/saved gid to become.
    pub gid: u32,
    /// Supplementary groups (from the host group database).
    pub groups: Vec<u32>,
}

/// Marker state: configured but not started.
pub struct Created;

/// Marker state: child process running.
pub struct Running {
    child_pid: Pid,
    tty: Option<TtyConnector>,
    forwarding: bool,
}

/// Marker state: child has exited.
pub struct Terminated {
    /// Exit code: `WIFEXITED` code, or `128 + signal` if signaled.
    status: i32,
}

/// A container in state `S` (typestate pattern: `Created` → `Running` →
/// `Terminated`). Holds no lock — run-lock lifecycle belongs to the `orca`
/// crate.
pub struct Container<S> {
    image: Image,
    session: SessionPaths,
    opts: NamespaceOpts,
    io: IoMode,
    cmd: Vec<String>,
    env: Vec<String>,
    cwd: PathBuf,
    run_as: Option<RunAs>,
    hostname: Option<String>,
    state: S,
}

impl Container<Created> {
    /// Start the container: create `session/`, `clone` the child (which
    /// mounts the overlay via `OverlayMount` and reports over the status
    /// pipe), then complete the fd handshake for tty mode.
    ///
    /// Precondition: the caller holds the environment's run lock.
    /// On failure everything created here is rolled back (the session
    /// directory is removed and the child, if any, is reaped).
    pub fn run(self) -> Result<Container<Running>, ContainerError> {
        let session_base = self.session.base.clone();
        std::fs::create_dir_all(&session_base).map_err(|source| ContainerError::Session {
            path: session_base.clone(),
            source,
        })?;

        match self.spawn() {
            Ok(running) => Ok(running),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&session_base);
                Err(e)
            }
        }
    }

    fn spawn(self) -> Result<Container<Running>, ContainerError> {
        use nix::fcntl::OFlag;
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

        // Status pipe: CLOEXEC on both ends; the write end auto-closes at
        // the child's exec, turning parent-side EOF into "success".
        let (status_r, status_w) =
            nix::unistd::pipe2(OFlag::O_CLOEXEC).map_err(syscall("pipe2"))?;
        // fd-passing socket (used in Tty mode; cheap to create always).
        let (sock_parent, sock_child) = socketpair(
            AddressFamily::Unix,
            SockType::Stream,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .map_err(syscall("socketpair"))?;

        let cfg = child::ChildConfig {
            image: &self.image,
            session: &self.session,
            io: self.io,
            cmd: &self.cmd,
            env: &self.env,
            cwd: &self.cwd,
            run_as: self.run_as.as_ref(),
            hostname: self.hostname.as_deref(),
            set_hostname: self.opts.uts,
            status_fd: status_w.as_raw_fd(),
            sock_fd: sock_child.as_raw_fd(),
        };

        let mut stack = vec![0u8; STACK_SIZE];
        let flags = build_clone_flags(&self.opts);
        let child_pid = {
            let cb: nix::sched::CloneCb = Box::new(|| child::child_main(&cfg));
            // SAFETY: the child callback only uses async-signal-safe-ish
            // operations on copied memory (no CLONE_VM) and ends in exec
            // or _exit; the stack outlives the clone call.
            unsafe { nix::sched::clone(cb, &mut stack, flags, Some(libc::SIGCHLD)) }
                .map_err(syscall("clone"))?
        };

        // Parent: close the child-side ends so EOF semantics work.
        drop(status_w);
        drop(sock_child);

        // Wait for the child to reach exec (EOF) or report a failed stage.
        if let Err(e) = parent::await_init(&status_r) {
            let _ = parent::reap(child_pid);
            return Err(e);
        }

        let (tty, forwarding) = match self.io {
            IoMode::Tty => {
                let master = match tty::recv_fd(&sock_parent) {
                    Ok(fd) => fd,
                    Err(e) => {
                        let _ = parent::reap(child_pid);
                        return Err(e);
                    }
                };
                let connector = match TtyConnector::start(master) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = parent::reap(child_pid);
                        return Err(e);
                    }
                };
                (Some(connector), false)
            }
            IoMode::Piped => {
                parent::forward_signals(child_pid)?;
                (None, true)
            }
        };

        Ok(Container {
            image: self.image,
            session: self.session,
            opts: self.opts,
            io: self.io,
            cmd: self.cmd,
            env: self.env,
            cwd: self.cwd,
            run_as: self.run_as,
            hostname: self.hostname,
            state: Running {
                child_pid,
                tty,
                forwarding,
            },
        })
    }
}

impl Container<Running> {
    /// Wait for the child to exit, tear down tty relay / signal
    /// forwarding, and remove the session directory (mounts died with the
    /// child's namespace; only directories remain).
    pub fn wait(self) -> Result<Container<Terminated>, ContainerError> {
        let status = parent::wait_child(self.state.child_pid)?;
        if let Some(tty) = self.state.tty {
            tty.wait()?;
        }
        if self.state.forwarding {
            parent::stop_forwarding();
        }
        let _ = std::fs::remove_dir_all(&self.session.base);
        Ok(Container {
            image: self.image,
            session: self.session,
            opts: self.opts,
            io: self.io,
            cmd: self.cmd,
            env: self.env,
            cwd: self.cwd,
            run_as: self.run_as,
            hostname: self.hostname,
            state: Terminated { status },
        })
    }

    /// PID of the container's init process (in the parent's PID
    /// namespace).
    pub fn child_pid(&self) -> Pid {
        self.state.child_pid
    }
}

impl Container<Terminated> {
    /// The image the container ran from.
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// Exit code (`WIFEXITED` code, or `128 + signo` when signaled).
    pub fn status(&self) -> i32 {
        self.state.status
    }
}

/// Translate namespace options into clone flags. `CLONE_NEWNS` is always
/// set; `SIGCHLD` is passed separately so `waitpid` works normally.
fn build_clone_flags(opts: &NamespaceOpts) -> CloneFlags {
    let mut flags = CloneFlags::CLONE_NEWNS;
    if opts.pid {
        flags |= CloneFlags::CLONE_NEWPID;
    }
    if opts.uts {
        flags |= CloneFlags::CLONE_NEWUTS;
    }
    if opts.ipc {
        flags |= CloneFlags::CLONE_NEWIPC;
    }
    if opts.net {
        flags |= CloneFlags::CLONE_NEWNET;
    }
    flags
}

/// Builder assembling a [`Container<Created>`]. `build()` has no side
/// effects and performs validation only: argv / env / cwd / run_as arrive
/// here already resolved by the `orca` crate's `ExecSpec` (policy); this
/// builder never consults the image config or the process environment.
pub struct ContainerBuilder {
    image: Image,
    session: SessionPaths,
    opts: NamespaceOpts,
    io: IoMode,
    cmd: Option<Vec<String>>,
    env: Vec<String>,
    cwd: Option<PathBuf>,
    run_as: Option<RunAs>,
    hostname: Option<String>,
}

impl ContainerBuilder {
    /// Start building a container for `image` with session paths prepared
    /// by the caller. Defaults: pid/uts/ipc unshared, network shared,
    /// piped IO, empty env, cwd `/`, no privilege drop.
    pub fn new(image: Image, session: SessionPaths) -> Self {
        Self {
            image,
            session,
            opts: NamespaceOpts::default(),
            io: IoMode::Piped,
            cmd: None,
            env: Vec::new(),
            cwd: None,
            run_as: None,
            hostname: None,
        }
    }

    /// Unshare (true) or share (false) the PID namespace.
    pub fn unshare_pid(mut self, v: bool) -> Self {
        self.opts.pid = v;
        self
    }

    /// Unshare (true) or share (false) the UTS namespace.
    pub fn unshare_uts(mut self, v: bool) -> Self {
        self.opts.uts = v;
        self
    }

    /// Unshare (true) or share (false) the IPC namespace.
    pub fn unshare_ipc(mut self, v: bool) -> Self {
        self.opts.ipc = v;
        self
    }

    /// Unshare (true) or share (false) the network namespace.
    pub fn unshare_net(mut self, v: bool) -> Self {
        self.opts.net = v;
        self
    }

    /// Set the IO mode (the CLI decides via `isatty` once, upstream).
    pub fn io(mut self, mode: IoMode) -> Self {
        self.io = mode;
        self
    }

    /// Set the resolved argv (required; `ExecSpec` supplies it).
    pub fn cmd(mut self, cmd: Vec<String>) -> Self {
        self.cmd = Some(cmd);
        self
    }

    /// Set the resolved environment (`KEY=VALUE` list, passed verbatim).
    pub fn env(mut self, env: Vec<String>) -> Self {
        self.env = env;
        self
    }

    /// Set the resolved working directory (falls back to `/` inside the
    /// container if it does not exist there).
    pub fn cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    /// Drop to this identity right before `exec` (default: stay as the
    /// invoking credentials, i.e. root).
    pub fn run_as(mut self, run_as: RunAs) -> Self {
        self.run_as = Some(run_as);
        self
    }

    /// Set the container hostname (applied when UTS is unshared;
    /// defaults to `"orca"`).
    pub fn hostname(mut self, name: &str) -> Self {
        self.hostname = Some(name.to_string());
        self
    }

    /// Validate and produce a [`Container<Created>`]. No side effects, no
    /// resolution. Fails with [`ContainerError::EmptyCommand`] if no
    /// command was set.
    pub fn build(self) -> Result<Container<Created>, ContainerError> {
        let cmd = match self.cmd {
            Some(c) if !c.is_empty() => c,
            _ => return Err(ContainerError::EmptyCommand),
        };
        Ok(Container {
            image: self.image,
            session: self.session,
            opts: self.opts,
            io: self.io,
            cmd,
            env: self.env,
            cwd: self.cwd.unwrap_or_else(|| PathBuf::from("/")),
            run_as: self.run_as,
            hostname: self.hostname,
            state: Created,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_flags_follow_opts() {
        let all = build_clone_flags(&NamespaceOpts {
            pid: true,
            uts: true,
            ipc: true,
            net: true,
        });
        assert!(all.contains(CloneFlags::CLONE_NEWNS));
        assert!(all.contains(CloneFlags::CLONE_NEWPID));
        assert!(all.contains(CloneFlags::CLONE_NEWNET));
        let min = build_clone_flags(&NamespaceOpts {
            pid: false,
            uts: false,
            ipc: false,
            net: false,
        });
        assert_eq!(min, CloneFlags::CLONE_NEWNS);
    }
}
