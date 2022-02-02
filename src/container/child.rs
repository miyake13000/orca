use crate::mount::{self, *};
use anyhow::{anyhow, Context, Result};
use core::convert::Infallible;
use nix::mount::MsFlags;
use nix::unistd;
use nix::unistd::geteuid;
use retry::{delay::Fixed, retry};
use std::ffi::{CStr, CString};
use std::fs::{self, copy, remove_dir};
use std::io::{stderr, stdin, stdout};
use std::os::unix::fs::symlink;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

pub struct Initializer;

const OLDROOT: &str = "oldroot";

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

    pub fn pivot_root<T: AsRef<Path>>(new_root: T) -> Result<()> {
        let old_root = PathBuf::from(new_root.as_ref()).join(OLDROOT);
        let mnt_args = MntArgs::new(
            FileAttr::Dir,
            Some(new_root.as_ref().to_str().unwrap()),
            new_root.as_ref().to_str().unwrap(),
            None,
            MsFlags::MS_BIND,
            None,
        );
        mount(mnt_args)
            .with_context(|| format!("Failed to bind mount '{}'", new_root.as_ref().display()))?;

        fs::create_dir_all(old_root.as_path())
            .with_context(|| format!("Failed to create '{}'", old_root.display()))?;
        unistd::pivot_root(new_root.as_ref(), old_root.as_path())
            .context("Failed to pivot_root")?;
        unistd::chdir("/").context("Failed to chdir to /")?;

        Ok(())
    }

    pub fn mount_mandatory_files() -> Result<()> {
        mount(mount::PROC).context("Failed to mount /proc")?;
        mount(mount::DEV).context("Failed to mount /dev")?;
        mount(mount::DEVPTS).context("Failed to mount /dev/pts")?;
        mount(mount::MQUEUE).context("Failed to mount /dev/mqueue")?;
        mount(mount::SHM).context("Failed to mount /dev/shm")?;
        mount(mount::DEVNULL).context("Failed to mount /dev/null")?;
        mount(mount::DEVRANDOM).context("Failed to mount /dev/random")?;
        mount(mount::DEVFULL).context("Failed to mount /dev/full")?;
        mount(mount::DEVTTY).context("Failed to mount /dev/tty")?;
        mount(mount::DEVZERO).context("Failed to mount /dev/zero")?;
        mount(mount::DEVURANDOM).context("Failed to mount /dev/urandom")?;
        //mount(mount::SYSFS).context("Failed to mount /sys")?;
        // trying to mount sysfs must fail with unknown reason

        Ok(())
    }

    pub fn create_ptmx_link() -> Result<()> {
        let ptmx = Path::new("/dev/ptmx");
        if !ptmx.exists() {
            symlink("pts/ptmx", ptmx).context("Failed to create symlink: /dev/ptmx -> pts/ptmx")?;
        }
        Ok(())
    }

    pub fn copy_resolv_conf() -> Result<()> {
        let host_resolvconf: PathBuf = PathBuf::from("/").join(OLDROOT).join("etc/resolv.conf");
        let resolvconf = Path::new("/etc/resolv.conf");
        copy(host_resolvconf.as_path(), resolvconf).with_context(|| {
            format!(
                "Failed to copy '{}' to '{}'",
                resolvconf.display(),
                resolvconf.display()
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

        mount(mount::DEVCONSOLE).context("Failed to mount /dev/console")?;

        Ok(())
    }

    pub fn unmount_old_root() -> Result<()> {
        let old_root = PathBuf::from("/").join(OLDROOT);
        umount(mount::OLDROOT).context("Failed to unmount /oldroot")?;
        remove_dir(old_root)?;
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
