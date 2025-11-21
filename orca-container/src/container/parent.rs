pub mod io_connector;

use anyhow::{Context, Result};
use io_connector::IoConnector;
use nix::pty::{grantpt, posix_openpt, unlockpt, PtyMaster};
use nix::sched::{self, CloneFlags};
use nix::unistd::Pid;
use std::fs::File;
use std::io::{stdin, stdout};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub struct Initilizer;

impl Initilizer {
    pub fn setns(child_pid: Pid, clone_flags: CloneFlags) -> Result<()> {
        let raw_child_pid = child_pid.as_raw() as isize;

        if clone_flags.contains(CloneFlags::CLONE_NEWUSER) {
            let userns_filename = format!("/proc/{}/ns/user", raw_child_pid);
            let userns = File::open(&userns_filename)
                .with_context(|| format!("Failed to open '{}", userns_filename))?;
            sched::setns(userns, CloneFlags::CLONE_NEWUSER).context("Failed to setns to userns")?;
        }

        if clone_flags.contains(CloneFlags::CLONE_NEWNS) {
            let mntns_filename = format!("/proc/{}/ns/mnt", raw_child_pid);
            let mntns = File::open(&mntns_filename)
                .with_context(|| format!("Failed to open '{}", mntns_filename))?;
            sched::setns(mntns, CloneFlags::CLONE_NEWNS).context("Failed to setns to mntns")?;
        }

        Ok(())
    }

    pub fn connect_tty() -> Result<IoConnector> {
        let pty_master = posix_openpt(nix::fcntl::OFlag::O_RDWR)
            .context("Child process has not connected tty yet")?;
        grantpt(&pty_master).context("Failed to grantpt")?;
        unlockpt(&pty_master).context("Failed to unlockpt")?;

        let stdout = stdout();
        let stdin = stdin();
        let pty_master_clone = unsafe { Self::clone_pty_master(&pty_master) };

        Ok(IoConnector::new(
            stdout,
            stdin,
            pty_master,
            pty_master_clone,
        ))
    }

    unsafe fn clone_pty_master(master: &PtyMaster) -> PtyMaster {
        unsafe {
            let fd = OwnedFd::from_raw_fd(master.as_raw_fd());
            PtyMaster::from_owned_fd(fd)
        }
    }
}
