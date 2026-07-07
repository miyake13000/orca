//! `orca run`: resolve the execution spec, take the run lock, start the
//! container, and propagate its exit code.

use std::io::IsTerminal;

use orca::{Env, ExecSpec, Invocation, LockFile, Workspace, resolve_run_target};
use orca_container::{ContainerBuilder, IoMode, SessionPaths};
use orca_image::BaseImageRef;

/// Namespace / identity flags and command from the CLI.
pub struct RunOpts {
    pub no_pid: bool,
    pub no_uts: bool,
    pub no_ipc: bool,
    pub network: bool,
    pub user: Option<String>,
    pub group: Option<String>,
    pub cmd: Vec<String>,
}

/// Run a container in `env`. Returns the child's exit code.
///
/// Lock lifecycle (DESIGN §5): acquired here before `Container::run`,
/// held until after `wait()`, released explicitly. Stale locks are
/// cleaned inside `acquire`. The container itself never sees the lock.
///
/// Policy resolution happens here (`Invocation` capture → run target →
/// `ExecSpec`); the container receives only resolved values.
pub fn run(env: &Env, opts: RunOpts) -> anyhow::Result<i32> {
    if !nix::unistd::geteuid().is_root() {
        anyhow::bail!("orca run requires root (try sudo, or a setuid-root install)");
    }

    // Assemble the image material (committed stack + base + config).
    let image = Workspace::open(env)?.image()?;

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
        &image.config,
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

    let mut builder = ContainerBuilder::new(image, session)
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
