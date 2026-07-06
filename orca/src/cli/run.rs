//! `orca run`: assemble the image, take the run lock, start the
//! container, and propagate its exit code.

use std::io::IsTerminal;

use orca::{Env, LockFile, Workspace};
use orca_container::{ContainerBuilder, IoMode, SessionPaths};

/// Namespace flags and command from the CLI.
pub struct RunOpts {
    pub no_pid: bool,
    pub no_uts: bool,
    pub no_ipc: bool,
    pub network: bool,
    pub cmd: Vec<String>,
}

/// Run a container in `env`. Returns the child's exit code.
///
/// Lock lifecycle (DESIGN §5): acquired here before `Container::run`,
/// held until after `wait()`, released explicitly. Stale locks are
/// cleaned inside `acquire`. The container itself never sees the lock.
pub fn run(env: &Env, opts: RunOpts) -> anyhow::Result<i32> {
    if !nix::unistd::geteuid().is_root() {
        anyhow::bail!("orca run requires root (try sudo)");
    }

    // Assemble the image material (committed stack + base + config).
    let image = Workspace::open(env)?.image()?;

    // Decide the IO mode once, before any terminal syscalls.
    let io = if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        IoMode::Tty
    } else {
        IoMode::Piped
    };

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
        .hostname(&env.name);
    if !opts.cmd.is_empty() {
        builder = builder.cmd(opts.cmd);
    }

    let result = builder.build()?.run()?.wait();
    lock.release()?;
    Ok(result?.status())
}
