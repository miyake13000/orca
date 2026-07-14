//! `orca run` policy: resolve the execution spec, take the run lock,
//! start the container, and propagate its exit code.
//!
//! Public entry: [`crate::Image::run`], which assembles the
//! [`ContainerImage`] and delegates here.

use std::io::IsTerminal;

use orca_container::{ContainerBuilder, ContainerError, IoMode, SessionPaths};
use orca_image::{BaseImageRef, ContainerImage};

use crate::env::Env;
use crate::exec_spec::{ExecSpec, ExecSpecError, Invocation, resolve_run_target};
use crate::image::ImageError;
use crate::lock::{LockError, LockFile};

/// Namespace / identity flags and command for a run.
pub struct RunOpts {
    /// Share the host PID namespace.
    pub no_pid: bool,
    /// Share the host UTS namespace.
    pub no_uts: bool,
    /// Share the host IPC namespace.
    pub no_ipc: bool,
    /// Isolate the network namespace (shared with the host by default).
    pub network: bool,
    /// Run as this user inside the container (uid or name).
    pub user: Option<String>,
    /// Run with this group inside the container (gid or name).
    pub group: Option<String>,
    /// Command and arguments (empty = the resolved default command).
    pub cmd: Vec<String>,
}

/// Errors from running a container.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// Mounting and pivoting need an effective uid of 0.
    #[error("orca run requires root (try sudo, or a setuid-root install)")]
    RootRequired,
    /// The mount material could not be assembled.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// The execution spec could not be resolved.
    #[error(transparent)]
    ExecSpec(#[from] ExecSpecError),
    /// The run lock could not be acquired or released.
    #[error(transparent)]
    Lock(#[from] LockError),
    /// The container failed to start, run, or be awaited.
    #[error(transparent)]
    Container(#[from] ContainerError),
}

/// Run a container in `env` from the assembled `material`. Returns the
/// child's exit code.
///
/// Lock lifecycle (DESIGN §5): acquired here before `Container::run`,
/// held until after `wait()`, released explicitly. Stale locks are
/// cleaned inside `acquire`. The container itself never sees the lock.
///
/// Policy resolution happens here (`Invocation` capture → run target →
/// `ExecSpec`); the container receives only resolved values.
pub(crate) fn run(
    env: &Env,
    material: ContainerImage,
    opts: RunOpts,
) -> Result<i32, RunError> {
    if !nix::unistd::geteuid().is_root() {
        return Err(RunError::RootRequired);
    }

    // Decide the IO mode once, before any terminal syscalls.
    let io = if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        IoMode::Tty
    } else {
        IoMode::Piped
    };

    // Resolve who to run as and the full execution spec (argv/env/cwd).
    let invocation = Invocation::capture()?;
    let target = resolve_run_target(
        &invocation,
        opts.user.as_deref(),
        opts.group.as_deref(),
    )?;
    let user_cmd = (!opts.cmd.is_empty()).then_some(opts.cmd);
    let spec = ExecSpec::resolve(
        matches!(env.base_ref, BaseImageRef::Host),
        &material.config,
        env.settings(),
        &invocation,
        target.as_ref(),
        user_cmd,
        io == IoMode::Tty,
    );

    let session = SessionPaths {
        base: env.session_path(),
        rootfs: env.rootfs_path(),
        work: env.work_path(),
        fake_rootfs: env.fake_rootfs_path(),
        fake_upper: env.fake_upper_path(),
        fake_work: env.fake_work_path(),
    };

    // Exclusion: refuse if a live container exists; hold the lock for the
    // container's whole lifetime.
    let lock = LockFile::acquire(&env.lock_path())?;

    let mut builder = ContainerBuilder::new(material, session)
        .io(io)
        .unshare_pid(!opts.no_pid)
        .unshare_uts(!opts.no_uts)
        .unshare_ipc(!opts.no_ipc)
        .unshare_net(opts.network)
        .hostname(&env.name)
        .cmd(spec.argv)
        .env(spec.env)
        .cwd(spec.cwd);
    if let Some(run_as) = spec.run_as {
        builder = builder.run_as(run_as);
    }

    let result = builder.build()?.run()?.wait();
    lock.release()?;
    Ok(result?.status())
}
