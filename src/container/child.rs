use std::ffi::CStr;
use std::fs;
use std::io::{stdin, stderr, stdout};
use std::os::unix::io::AsRawFd;
use nix::mount;
use nix::unistd;
use retry::{retry, delay::Fixed};

pub struct Child {
    rootfs_path: String,
}

impl Child {
    pub fn new(rootfs_path: String) -> Self {
        Child {
            rootfs_path
        }
    }

    pub fn pivot_root(&self) -> std::result::Result<(), ()> {
        let oldroot_path = format!("{}/oldroot", self.rootfs_path);

        mount::mount::<str, str, str, str>(
            Some(&self.rootfs_path),
            &self.rootfs_path,
            None,
            mount::MsFlags::MS_BIND,
            None
            ).unwrap();

        fs::create_dir_all(&oldroot_path).unwrap();
        unistd::pivot_root(self.rootfs_path.as_str(), oldroot_path.as_str()).unwrap();

        mount::umount2("/oldroot", mount::MntFlags::MNT_DETACH).unwrap();
        fs::remove_dir("/oldroot").unwrap();

        unistd::chdir("/").unwrap();

        Ok(())
    }

    pub fn mount(&self) -> std::result::Result<(), ()> {
        let procfs_path = "/proc";
        let devpts_path = "/dev/pts";
        fs::create_dir_all(procfs_path).unwrap();
        fs::create_dir_all(devpts_path).unwrap();

        mount::mount::<str, str, str, str>(
            None,
            procfs_path,
            Some("proc"),
            mount::MsFlags::empty(),
            None
            ).unwrap();
        mount::mount::<str, str, str, str>(
            None,
            devpts_path,
            Some("devpts"),
            mount::MsFlags::empty(),
            None
            ).unwrap();

        Ok(())
    }

    pub fn sethostname(&self, new_hostname: &str) -> std::result::Result<(), ()> {
        unistd::sethostname(new_hostname).unwrap();
        Ok(())
    }

    pub fn connect_tty(&self) -> std::result::Result<(), ()> {
        let _ = unistd::setsid().unwrap();

        //let err_with_nothing: std::result::Result<RawFd, ()> = Err(());

        // If /dev/pts/0 is not found, return Err(())
        let pty_slave = retry(Fixed::from_millis(10).take(100), || {
            nix::fcntl::open(
                "/dev/pts/0",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty()
            )
        }).unwrap();

        let pty_slave_fd = pty_slave.as_raw_fd();
        let stdout = stdout().as_raw_fd();
        let stderr = stderr().as_raw_fd();
        let stdin  = stdin().as_raw_fd();

        let _ = unistd::dup2(pty_slave_fd, stdout).unwrap();
        let _ = unistd::dup2(pty_slave_fd, stderr).unwrap();
        let _ = unistd::dup2(pty_slave_fd, stdin).unwrap();

        Ok(())
    }

    pub fn exec(self, command: &CStr, argv: &Vec<&CStr>, envp: &Vec<&CStr>) {
        let _ = unistd::execvpe(command, argv, envp); // never return value
    }
}
