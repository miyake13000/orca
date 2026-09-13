//! `orca run` policy: resolve the execution spec, take the run lock,
//! start the container, resolve execution in its rootfs, and propagate its exit code.
//!
//! Public entry: [`crate::Image::run`], which assembles the
//! [`ContainerImage`] and delegates here.

use std::io::IsTerminal;

use orca_container::{ContainerBuilder, ContainerError, IoMode, SessionPaths};
use orca_image::{BaseImageRef, ContainerImage};

use crate::config::{ConfigError, Settings};
use crate::env::Env;
use crate::image::ImageError;
use crate::lock::{LockError, LockFile};

/// Resolved global/env/argument overrides for one execution.
pub type RunOpts = Settings;

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
    ExecSpec(#[from] ConfigError),
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
/// Settings are already merged by Config. Only cwd and TERM are captured;
/// account resolution and environment assembly execute in the child rootfs.
pub(crate) fn run(env: &Env, material: ContainerImage, opts: RunOpts) -> Result<i32, RunError> {
    if !nix::unistd::geteuid().is_root() {
        return Err(RunError::RootRequired);
    }

    // Decide the IO mode once, before any terminal syscalls.
    let io = if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        IoMode::Tty
    } else {
        IoMode::Piped
    };

    opts.validate()?;
    let host = matches!(env.base_ref, BaseImageRef::Host);
    let invocation_cwd = if host && opts.working_dir.is_none() {
        std::env::current_dir().map_err(|e| ConfigError::Invalid(e.to_string()))?
    } else {
        std::path::PathBuf::from("/")
    };
    let term = std::env::var("TERM").ok();
    let config = material.config.clone();
    let ns = opts.namespaces.clone();
    let resolver = std::sync::Arc::new(move || {
        crate::exec_spec::resolve(
            &opts,
            &config,
            host,
            &invocation_cwd,
            term.as_deref(),
            io == IoMode::Tty,
        )
    });

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

    let builder = ContainerBuilder::new(material, session)
        .io(io)
        .unshare_pid(ns.pid.unwrap_or(true))
        .unshare_uts(ns.uts.unwrap_or(true))
        .unshare_ipc(ns.ipc.unwrap_or(true))
        .unshare_net(ns.network.unwrap_or(false))
        .hostname(&env.name)
        .resolve_exec(resolver);

    let result = builder.build()?.run()?.wait();
    lock.release()?;
    Ok(result?.status())
}
