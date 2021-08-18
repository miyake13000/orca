use std::ffi::CStr;
use std::ops::BitOr;
use std::fs;
use std::io::{stdin, stderr, stdout};
use std::os::unix::io::AsRawFd;
use nix::mount::{mount, umount2, MsFlags, MntFlags};
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

        mount::<str, str, str, str>(
            Some(&self.rootfs_path),
            &self.rootfs_path,
            None,
            MsFlags::MS_BIND,
            None
            ).unwrap();

        fs::create_dir_all(&oldroot_path).unwrap();
        unistd::pivot_root(self.rootfs_path.as_str(), oldroot_path.as_str()).unwrap();

        umount2("/oldroot", MntFlags::MNT_DETACH).unwrap();
        fs::remove_dir("/oldroot").unwrap();

        unistd::chdir("/").unwrap();

        Ok(())
    }

    pub fn mount(&self) -> std::result::Result<(), ()> {

        let procfs_path = "/proc";
        fs::create_dir_all(procfs_path).unwrap();
        mount::<str, str, str, str>(
            Some("proc"),
            procfs_path,
            Some("proc"),
            MsFlags::empty()
                .bitor(MsFlags::MS_NOSUID)
                .bitor(MsFlags::MS_NODEV)
                .bitor(MsFlags::MS_NOEXEC),
            None
        ).unwrap();

        let dev_path = "/dev";
        fs::create_dir_all(dev_path).unwrap();
        mount::<str, str, str, str>(
            Some("tmpfs"),
            dev_path,
            Some("tmpfs"),
            MsFlags::empty().bitor(MsFlags::MS_NOSUID),
            Some("size=65536k,mode=755")
        ).unwrap();

        let devpts_path = "/dev/pts";
        fs::create_dir_all(devpts_path).unwrap();
        mount::<str, str, str, str>(
            Some("devpts"),
            devpts_path,
            Some("devpts"),
            MsFlags::empty()
                .bitor(MsFlags::MS_NOSUID)
                .bitor(MsFlags::MS_NOEXEC),
            Some("mode=620,ptmxmode=666")
        ).unwrap();

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
