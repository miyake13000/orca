use crate::mount::*;
use anyhow::{anyhow, Context, Result};
use const_format::concatcp;
use core::convert::Infallible;
use nix::unistd;
use nix::unistd::geteuid;
use retry::{delay::Fixed, retry};
use std::ffi::{CStr, CString};
use std::fs::{self, copy, create_dir_all};
use std::io::{stderr, stdin, stdout};
use std::os::unix::fs::symlink;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

pub struct Initializer;

const OLDROOT_NAME: &str = "oldroot";

impl Initializer {
    pub fn wait_for_mapping_id() -> Result<()> {
        retry(Fixed::from_millis(50).take(20), || {
            let uid = geteuid().as_raw() as u32;
            match uid {
                0 => Ok(()),
                _ => Err(()),
            }
        })
        .map_err(|_| anyhow!("Time out to wait for mapping UID"))
    }

    pub fn store_resolv_conf<T: AsRef<Path>>(temp_dir: T) -> Result<()> {
        create_dir_all(temp_dir.as_ref())
            .with_context(|| format!("Failed to mkdir '{}'", temp_dir.as_ref().display()))?;
        let stored_resolvconf = PathBuf::from(temp_dir.as_ref()).join("resolv.conf");
        copy("/etc/resolv.conf", stored_resolvconf).with_context(|| {
            format!(
                "Failed to copy '/etc/resolv.conf' to {}",
                temp_dir.as_ref().display()
            )
        })?;
        Ok(())
    }

    pub fn pivot_root<T: Into<PathBuf>>(new_root: T) -> Result<()> {
        let new_root = new_root.into();
        let old_root = new_root.join(OLDROOT_NAME);

        fs::create_dir_all(old_root.as_path())
            .with_context(|| format!("Failed to create '{}'", old_root.display()))?;
        unistd::pivot_root(new_root.as_path(), old_root.as_path())
            .context("Failed to pivot_root")?;
        unistd::chdir("/").context("Failed to chdir to /")?;

        Ok(())
    }

    pub fn mount_mandatory_files() -> Result<()> {
        Mount::new("proc", FileType::Dir)
            .src("proc")
            .fs_type("proc")
            .flags(MountFlags::MS_NODEV | MountFlags::MS_NOSUID | MountFlags::MS_NOEXEC)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::new("/dev", FileType::Dir)
            .src("tmpfs")
            .fs_type("tmpfs")
            .flags(MountFlags::MS_NOSUID)
            .data("mode=755")
            .mount()
            .context("Failed to mount /proc")?;

        Mount::new("/dev/pts", FileType::Dir)
            .src("devpts")
            .fs_type("devpts")
            .flags(MountFlags::MS_NOSUID | MountFlags::MS_NOEXEC)
            .data("mode=620,ptmxmode=666")
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/mqueue", FileType::Dir)
            .src("mqueue")
            .fs_type("mqueue")
            .flags(MountFlags::MS_NOSUID | MountFlags::MS_NODEV | MountFlags::MS_NOEXEC)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/shm", FileType::Dir)
            .src("shm")
            .fs_type("tmpfs")
            .flags(MountFlags::MS_NOSUID | MountFlags::MS_NODEV | MountFlags::MS_NOEXEC)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/null", FileType::File)
            .src(concatcp!("/", OLDROOT_NAME, "/dev/null"))
            .flags(MountFlags::MS_BIND)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/random", FileType::File)
            .src(concatcp!("/", OLDROOT_NAME, "/dev/random"))
            .flags(MountFlags::MS_BIND)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/full", FileType::File)
            .src(concatcp!("/", OLDROOT_NAME, "/dev/full"))
            .flags(MountFlags::MS_BIND)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/tty", FileType::File)
            .src(concatcp!("/", OLDROOT_NAME, "/dev/tty"))
            .flags(MountFlags::MS_BIND)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/zero", FileType::File)
            .src(concatcp!("/", OLDROOT_NAME, "/dev/zero"))
            .flags(MountFlags::MS_BIND)
            .mount()
            .context("Failed to mount /proc")?;

        Mount::<_, &str>::new("/dev/urandom", FileType::File)
            .src(concatcp!("/", OLDROOT_NAME, "/dev/urandom"))
            .flags(MountFlags::MS_BIND)
            .mount()
            .context("Failed to mount /proc")?;

        //BUG: trying to mount sysfs must fail with unknown reason
        //MountArgs::new("/sys", FileType::Dir)
        //    .src("sysfs")
        //    .fs_type("sysfs")
        //    .add_flags(MountFlags::MS_RDONLY)
        //    .add_flags(MountFlags::MS_NOSUID)
        //    .add_flags(MountFlags::MS_NODEV)
        //    .add_flags(MountFlags::MS_NOEXEC)
        //    .mount()
        //    .context("Failed to mount /proc")?;

        Ok(())
    }

    pub fn create_ptmx_link() -> Result<()> {
        let ptmx = Path::new("/dev/ptmx");
        if !ptmx.exists() {
            symlink("pts/ptmx", ptmx).context("Failed to create symlink: /dev/ptmx -> pts/ptmx")?;
        }
        Ok(())
    }

    pub fn copy_resolv_conf<T: AsRef<Path>>(stored_dir: T) -> Result<()> {
        let stored_dir = PathBuf::from(stored_dir.as_ref());
        let stored_resolvconf = stored_dir.strip_prefix("/")?.join("resolv.conf");
        let stored_resolvconf = PathBuf::from("/").join("oldroot").join(stored_resolvconf);
        let resolvconf_path = "/etc/resolv.conf";
        copy(stored_resolvconf.as_path(), resolvconf_path).with_context(|| {
            format!(
                "Failed to copy '{}' to '{}'",
                stored_resolvconf.display(),
                resolvconf_path
            )
        })?;
        Ok(())
    }

    pub fn set_hostname<T: AsRef<str>>(new_hostname: T) -> Result<()> {
        unistd::sethostname(new_hostname.as_ref())?;
        Ok(())
    }

    pub fn connect_tty() -> Result<()> {
        let _ = unistd::setsid().unwrap();
        let pty_slave = retry(Fixed::from_millis(10).take(100), || {
            nix::fcntl::open(
                "/dev/pts/0",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty(),
            )
        })
        .context("Failed to open /dev/pts/0")?;

        let pty_slave_fd = pty_slave.as_raw_fd();
        let stdout = stdout().as_raw_fd();
        let stderr = stderr().as_raw_fd();
        let stdin = stdin().as_raw_fd();

        let _ = unistd::dup2(pty_slave_fd, stdout)?;
        let _ = unistd::dup2(pty_slave_fd, stderr)?;
        let _ = unistd::dup2(pty_slave_fd, stdin)?;

        Mount::<_, &str>::new("/dev/console", FileType::File)
            .src(concatcp!("/", OLDROOT_NAME, "/dev/console"))
            .flags(MountFlags::MS_BIND)
            .mount()
            .context("Failed to mount /dev/console")?;

        Ok(())
    }

    pub fn unmount_old_root() -> Result<()> {
        UnMount::new(concatcp!("/", OLDROOT_NAME))
            .remove_mount_point(true)
            .flags(UnMountFlags::MNT_DETACH)
            .unmount()
            .context("Failed to unmount /oldroot")?;

        Ok(())
    }

    pub fn exec<S, T>(command: S, args: Option<Vec<T>>, envp: &[&CStr]) -> Result<Infallible>
    where
        S: AsRef<str>,
        T: AsRef<str>,
    {
        let command =
            CString::new(command.as_ref()).context("Failed to change command into CSting")?;

        let mut argv: Vec<CString> = vec![command.clone()];
        if let Some(args_vec) = args {
            let args_iter = args_vec.iter();
            for arg in args_iter {
                let arg_cstring =
                    CString::new(arg.as_ref()).context("Failed to change arg into CString")?;
                argv.push(arg_cstring);
            }
        }

        unistd::execvpe(command.as_c_str(), &argv, envp)
            .with_context(|| format!("Not found: '{}'", command.to_str().unwrap()))
    }
}
