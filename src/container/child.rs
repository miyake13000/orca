use crate::mount::{self, *};
use nix::mount::MsFlags;
use nix::unistd;
use retry::{delay::Fixed, retry};
use std::ffi::CStr;
use std::fs;
use std::io::{stderr, stdin, stdout};
use std::os::unix::io::AsRawFd;

pub struct Child {
    rootfs_path: String,
}

impl Child {
    pub fn new(rootfs_path: String) -> Self {
        Child { rootfs_path }
    }

    pub fn pivot_root(&self) -> std::result::Result<(), ()> {
        let oldroot_path = format!("{}/oldroot", self.rootfs_path);

        let mnt_args = MntArgs::new(
            FileAttr::Dir,
            Some(&self.rootfs_path),
            &self.rootfs_path,
            None,
            MsFlags::MS_BIND,
            None,
        );
        mount(mnt_args).unwrap();

        fs::create_dir_all(&oldroot_path).unwrap();
        unistd::pivot_root(self.rootfs_path.as_str(), oldroot_path.as_str()).unwrap();
        unistd::chdir("/").unwrap();

        Ok(())
    }

    pub fn mount_all(&self) -> std::result::Result<(), ()> {
        mount(mount::PROC)?;
        mount(mount::DEV)?;
        mount(mount::DEVPTS)?;
        //mount(mount::SYSFS)?; // Cannot mount because netns isnt separated
        mount(mount::MQUEUE)?;
        mount(mount::SHM)?;
        mount(mount::DEVNULL)?;
        mount(mount::DEVRANDOM)?;
        mount(mount::DEVFULL)?;
        mount(mount::DEVTTY)?;
        mount(mount::DEVZERO)?;
        mount(mount::DEVURANDOM)?;

        umount(mount::OLDROOT)?;

        Ok(())
    }

    pub fn sethostname(&self, new_hostname: &str) -> std::result::Result<(), ()> {
        unistd::sethostname(new_hostname).unwrap();
        Ok(())
    }

    pub fn connect_tty(&self) -> std::result::Result<(), ()> {
        let _ = unistd::setsid().unwrap();

        let pty_slave = retry(Fixed::from_millis(10).take(100), || {
            nix::fcntl::open(
                "/dev/pts/0",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty(),
            )
        })
        .unwrap();

        mount(mount::DEVCONSOLE)?;

        let pty_slave_fd = pty_slave.as_raw_fd();
        let stdout = stdout().as_raw_fd();
        let stderr = stderr().as_raw_fd();
        let stdin = stdin().as_raw_fd();

        let _ = unistd::dup2(pty_slave_fd, stdout).unwrap();
        let _ = unistd::dup2(pty_slave_fd, stderr).unwrap();
        let _ = unistd::dup2(pty_slave_fd, stdin).unwrap();

        Ok(())
    }

    pub fn exec(self, command: &CStr, argv: &Vec<&CStr>, envp: &Vec<&CStr>) {
        let _ = unistd::execvpe(command, argv, envp); // never return value
    }
}
