pub mod io_connector;
mod mapping_id;

use crate::command::Command;
use anyhow::{anyhow, Context, Result};
use io_connector::IoConnector;
use libc::{grantpt, unlockpt};
use mapping_id::{IDType, MappingID};
use nix::sched::{self, CloneFlags};
use nix::unistd::Pid;
use retry::{delay::Fixed, retry};
use std::fs::File;
use std::io::{stdin, stdout, Write};
use std::os::unix::io::AsRawFd;

pub struct Initilizer;

impl Initilizer {
    pub fn setns(child_pid: Pid, clone_flags: CloneFlags) -> Result<()> {
        let raw_child_pid = child_pid.as_raw() as isize;

        if clone_flags.contains(CloneFlags::CLONE_NEWUSER) {
            let userns_filename = format!("/proc/{}/ns/user", raw_child_pid);
            let userns = File::open(&userns_filename)
                .with_context(|| format!("Failed to open '{}", userns_filename))?;
            let userns_fd = userns.as_raw_fd();
            sched::setns(userns_fd, CloneFlags::CLONE_NEWUSER)
                .context("Failed to setns to userns")?;
        }

        if clone_flags.contains(CloneFlags::CLONE_NEWNS) {
            let mntns_filename = format!("/proc/{}/ns/mnt", raw_child_pid);
            let mntns = File::open(&mntns_filename)
                .with_context(|| format!("Failed to open '{}", mntns_filename))?;
            let mntns_fd = mntns.as_raw_fd();
            sched::setns(mntns_fd, CloneFlags::CLONE_NEWNS).context("Failed to setns to mntns")?;
        }

        Ok(())
    }
    pub fn map_id(child_pid: Pid) -> Result<()> {
        let mapping_uid = MappingID::create(IDType::UID)?;
        let mapping_gid = MappingID::create(IDType::GID)?;

        let uid_map_path = format!("/proc/{}/uid_map", child_pid);
        let gid_map_path = format!("/proc/{}/gid_map", child_pid);
        let setgroups_path = format!("/proc/{}/setgroups", child_pid);

        File::options()
            .append(true)
            .open(&uid_map_path)
            .with_context(|| format!("Failed to open '{}'", uid_map_path))?
            .write_all(&mapping_uid.to_string().into_bytes())
            .with_context(|| format!("Faield to write to '{}", uid_map_path))?;

        File::options()
            .append(true)
            .open(&setgroups_path)
            .with_context(|| format!("Faield to open '{}", setgroups_path))?
            .write_all(b"deny")
            .with_context(|| format!(" Failed to write to '{}", setgroups_path))?;

        File::options()
            .append(true)
            .open(&gid_map_path)
            .with_context(|| format!("Failed to open '{}", gid_map_path))?
            .write_all(&mapping_gid.to_string().into_bytes())
            .with_context(|| format!("Faield to write to '{}", gid_map_path))?;

        Ok(())
    }

    pub fn map_id_with_subuid(child_pid: Pid) -> Result<()> {
        let mut args_uidmap: Vec<String> = vec![child_pid.to_string()];
        let mut args_gidmap: Vec<String> = vec![child_pid.to_string()];

        let mapping_uid = MappingID::create(IDType::UID)?;
        let mapping_gid = MappingID::create(IDType::GID)?;
        let mapping_subuid = MappingID::create(IDType::SUBUID)?;
        let mapping_subgid = MappingID::create(IDType::SUBGID)?;

        let _ = args_uidmap.append(&mut mapping_uid.into_vec());
        let _ = args_gidmap.append(&mut mapping_gid.into_vec());
        let _ = args_uidmap.append(&mut mapping_subuid.into_vec());
        let _ = args_gidmap.append(&mut mapping_subgid.into_vec());

        let newuidmap_status = Command::new("newuidmap", Some(args_uidmap)).execute()?;
        match newuidmap_status {
            Some(code) if code < 0 => return Err(anyhow!("newuidmap is exited with {}", code)),
            None => return Err(anyhow!("newuidmap is exited with no status")),
            _ => {}
        }
        let newgidmap_status = Command::new("newgidmap", Some(args_gidmap)).execute()?;
        match newgidmap_status {
            Some(code) if code < 0 => return Err(anyhow!("newgidmap is exited with {}", code)),
            None => return Err(anyhow!("newgidmap is exited with no status")),
            _ => {}
        }

        Ok(())
    }

    pub fn connect_tty() -> Result<IoConnector> {
        let pty_master_path = "/dev/pts/ptmx";

        let pty_master = retry(Fixed::from_millis(50).take(20), || {
            nix::fcntl::open(
                pty_master_path,
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::all(),
            )
        })
        .with_context(|| format!("Failed to open '{}'", pty_master_path))?;

        if unsafe { grantpt(pty_master) } < 0 {
            return Err(anyhow!("Failed to grantpt('{}')", pty_master_path));
        }
        if unsafe { unlockpt(pty_master) } < 0 {
            return Err(anyhow!("Failed to unlockpt('{}')", pty_master_path));
        }

        Ok(IoConnector::new(
            stdout().as_raw_fd(),
            stdin().as_raw_fd(),
            pty_master,
            pty_master,
        ))
    }
}
