pub mod io_connector;

use anyhow::{anyhow, Context, Result};
use io_connector::IoConnector;
use nix::pty::{grantpt, unlockpt, PtyMaster};
use nix::sched::{self, CloneFlags};
use nix::unistd::Pid;
use std::fs::File;
use std::io::{stdin, stdout};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::io::AsRawFd;

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
        let pty_master_path = "/dev/pts/ptmx";
        let pty_master = nix::fcntl::open(
            pty_master_path,
            nix::fcntl::OFlag::O_RDWR,
            nix::sys::stat::Mode::all(),
        )
        .context("Child process has not connected tty yet")?;

        let pty_master = unsafe { PtyMaster::from_owned_fd(pty_master) };
        if grantpt(&pty_master).is_err() {
            return Err(anyhow!("Failed to grantpt('{}')", pty_master_path));
        }
        if unlockpt(&pty_master).is_err() {
            return Err(anyhow!("Failed to unlockpt('{}')", pty_master_path));
        }

        let stdout = unsafe { OwnedFd::from_raw_fd(stdout().as_raw_fd()) };
        let stdin = unsafe { OwnedFd::from_raw_fd(stdin().as_raw_fd()) };
        let pty_master: OwnedFd = pty_master.into();
        let child_stdout = pty_master.try_clone().unwrap();
        let child_stdin = pty_master;

        Ok(IoConnector::new(stdout, stdin, child_stdout, child_stdin))
    }
}
