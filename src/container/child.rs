use crate::mount::{self, *};
use anyhow::{Context, Result};
use core::convert::Infallible;
use nix::mount::MsFlags;
use nix::unistd;
use retry::{delay::Fixed, retry};
use std::ffi::CStr;
use std::fs;
use std::io::{stderr, stdin, stdout};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

pub struct Child {
    rootfs_path: PathBuf,
}

impl Child {
    pub fn new(rootfs_path: PathBuf) -> Self {
        Child { rootfs_path }
    }

    pub fn pivot_root(&self) -> Result<()> {
        let oldroot_path = self.rootfs_path.join("oldroot");

        let mnt_args = MntArgs::new(
            FileAttr::Dir,
            Some(&self.rootfs_path.to_str().unwrap()),
            &self.rootfs_path.to_str().unwrap(),
            None,
            MsFlags::MS_BIND,
            None,
        );
        mount(mnt_args)
            .with_context(|| format!("Failed to bind mount '{}'", self.rootfs_path.display()))?;

        fs::create_dir_all(&oldroot_path)
            .with_context(|| format!("Failed to create '{}'", oldroot_path.display()))?;
        unistd::pivot_root(&self.rootfs_path, &oldroot_path).context("Failed to pivot_root")?;
        unistd::chdir("/").context("Failed to chdir to /")?;

        Ok(())
    }

    pub fn mount_all(&self) -> Result<()> {
        mount(mount::PROC).context("Failed to mount /proc")?;
        mount(mount::DEV).context("Failed to mount /dev")?;
        mount(mount::DEVPTS).context("Failed to mount /dev/pts")?;
        //mount(mount::SYSFS).context("Failed to mount /sys")?; // Cannot mount because netns isnt separated
        mount(mount::MQUEUE).context("Failed to mount /dev/mqueue")?;
        mount(mount::SHM).context("Failed to mount /dev/shm")?;
        mount(mount::DEVNULL).context("Failed to mount /dev/null")?;
        mount(mount::DEVRANDOM).context("Failed to mount /dev/random")?;
        mount(mount::DEVFULL).context("Failed to mount /dev/full")?;
        mount(mount::DEVTTY).context("Failed to mount /dev/tty")?;
        mount(mount::DEVZERO).context("Failed to mount /dev/zero")?;
        mount(mount::DEVURANDOM).context("Failed to mount /dev/urandom")?;

        umount(mount::OLDROOT).context("Failed to unmount /oldroot")?;

        Ok(())
    }

    pub fn sethostname(&self, new_hostname: &str) -> Result<()> {
        unistd::sethostname(new_hostname)?;
        Ok(())
    }

    pub fn connect_tty(&self) -> Result<()> {
        let _ = unistd::setsid().unwrap();

        let pty_slave = retry(Fixed::from_millis(10).take(100), || {
            nix::fcntl::open(
                "/dev/pts/0",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty(),
            )
        })
        .context("Failed to open /dev/pts/0")?;

        mount(mount::DEVCONSOLE).context("Failed to mount /dev/console")?;

        let pty_slave_fd = pty_slave.as_raw_fd();
        let stdout = stdout().as_raw_fd();
        let stderr = stderr().as_raw_fd();
        let stdin = stdin().as_raw_fd();

        let _ = unistd::dup2(pty_slave_fd, stdout)?;
        let _ = unistd::dup2(pty_slave_fd, stderr)?;
        let _ = unistd::dup2(pty_slave_fd, stdin)?;

        Ok(())
    }

    pub fn exec(self, command: &CStr, argv: &Vec<&CStr>, envp: &Vec<&CStr>) -> Result<Infallible> {
        unistd::execvpe(command, argv, envp)
            .with_context(|| format!("Not found: '{}'", command.to_str().unwrap()))
    }
}
