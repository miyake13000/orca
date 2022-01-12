mod child;
mod id;

use crate::command::Command;
use crate::image::Image;
use crate::STACK_SIZE;
use anyhow::{anyhow, Context, Result};
use child::Child;
use id::{IdType, MappingId};
use libc::{grantpt, unlockpt};
use nix::sched::{clone, setns, CloneFlags};
use nix::sys::wait::wait;
use nix::unistd::{geteuid, read, write, Pid};
use retry::{delay::Fixed, retry};
use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io::{stdin, stdout, Write};
use std::os::unix::io::AsRawFd;
use std::thread;

pub struct Container {
    image: Image,
    child_pid: Pid,
}

impl Container {
    pub fn new(
        image: Image,
        command: String,
        cmd_args: Option<Vec<String>>,
        netns_flag: bool,
    ) -> Result<Self> {
        let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];
        let cb = Box::new(|| {
            Self::child_main(&command, &cmd_args, &image.rootfs_path, &image.image_name)
        });
        let signals = Some(libc::SIGCHLD);

        let mut flags = CloneFlags::empty();
        flags.insert(CloneFlags::CLONE_NEWUSER);
        flags.insert(CloneFlags::CLONE_NEWNS);
        flags.insert(CloneFlags::CLONE_NEWUTS);
        flags.insert(CloneFlags::CLONE_NEWPID);
        flags.insert(CloneFlags::CLONE_NEWIPC);
        if netns_flag {
            flags.insert(CloneFlags::CLONE_NEWNET);
        }

        let child_pid =
            clone(cb, stack, flags, signals).context("Failed to clone child process")?;

        Ok(Container { image, child_pid })
    }

    pub fn map_id(&self) -> Result<()> {
        let mapping_uid = MappingId::new(IdType::Uid)?;
        let mapping_gid = MappingId::new(IdType::Gid)?;

        let uid_map_path = format!("/proc/{}/uid_map", self.child_pid);
        let gid_map_path = format!("/proc/{}/gid_map", self.child_pid);
        let setgroups_path = format!("/proc/{}/setgroups", self.child_pid);

        OpenOptions::new()
            .append(true)
            .open(&uid_map_path)
            .with_context(|| format!("Failed to open '{}'", uid_map_path))?
            .write_all(&mapping_uid.to_string().into_bytes())
            .with_context(|| format!("Faield to write to '{}", uid_map_path))?;

        OpenOptions::new()
            .append(true)
            .open(&setgroups_path)
            .with_context(|| format!("Faield to open '{}", setgroups_path))?
            .write_all(b"deny")
            .with_context(|| format!(" Failed to write to '{}", setgroups_path))?;

        OpenOptions::new()
            .append(true)
            .open(&gid_map_path)
            .with_context(|| format!("Failed to open '{}", gid_map_path))?
            .write_all(&mapping_gid.to_string().into_bytes())
            .with_context(|| format!("Faield to write to '{}", gid_map_path))?;

        Ok(())
    }

    pub fn map_id_with_subuid(&self) -> Result<()> {
        let mut args_uidmap: Vec<String> = vec![self.child_pid.to_string()];
        let mut args_gidmap: Vec<String> = vec![self.child_pid.to_string()];

        let mapping_uid = MappingId::new(IdType::Uid)?;
        let mapping_gid = MappingId::new(IdType::Gid)?;
        let mapping_subuid = MappingId::new(IdType::SubUid)?;
        let mapping_subgid = MappingId::new(IdType::SubGid)?;

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

    pub fn connect_tty(&self) -> Result<()> {
        Self::setns(self.child_pid).context("Faield to setns")?;
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

        thread::spawn(move || {
            let stdout = stdout().as_raw_fd();
            let mut s: [u8; 1] = [0; 1];
            loop {
                if read(pty_master, &mut s).is_err() {
                    return;
                };
                if write(stdout, &s).is_err() {
                    return;
                };
            }
        });

        thread::spawn(move || {
            let stdin = stdin().as_raw_fd();
            let mut s: [u8; 1] = [0; 1];
            loop {
                if nix::unistd::read(stdin, &mut s).is_err() {
                    return;
                }
                if nix::unistd::write(pty_master, &s).is_err() {
                    return;
                }
            }
        });

        Ok(())
    }

    pub fn wait(self) -> Result<Image> {
        wait().context("Failed to wait child process")?;
        Ok(self.image)
    }

    fn child_main(
        command: &str,
        cmd_args: &Option<Vec<String>>,
        path_rootfs: &str,
        image_name: &str,
    ) -> isize {
        retry(Fixed::from_millis(50).take(20), || {
            let uid = geteuid().as_raw() as u32;
            match uid {
                0 => Ok(()),
                _ => Err(()),
            }
        })
        .expect("Failed to uid mapping");

        let child = Child::new(path_rootfs.to_string());
        child.pivot_root().context("Failed to pivot_root").unwrap();
        child.mount_all().context("Failed to mount").unwrap();
        child
            .sethostname(image_name)
            .context("Failed to sethostname")
            .unwrap();
        child
            .connect_tty()
            .context("Failed to connect_tty")
            .unwrap();

        // convert command: String -> command_cstr: CStr
        let command_cstring = CString::new(command)
            .context("Failed to change command into CSting")
            .unwrap();
        let command_cstr = command_cstring.as_c_str();

        // convert cmd_args: Vec<String> -> cmd_args_cstring: Vec<CString>
        let mut cmd_args_cstring: Vec<CString> = Vec::new();
        let cmd_args = cmd_args.clone();
        if let Some(args) = cmd_args {
            let cmd_args_iter = args.iter();
            for arg in cmd_args_iter {
                let arg_cstring = CString::new(arg.as_str())
                    .context("Failed to change arg into CString")
                    .unwrap();
                cmd_args_cstring.push(arg_cstring);
            }
        }

        // create argv
        let mut argv: Vec<&CStr> = vec![command_cstr];
        for arg in cmd_args_cstring.iter() {
            argv.push(arg.as_c_str());
        }

        //create envp
        let envp: Vec<&CStr> = vec![
            CStr::from_bytes_with_nul(b"SHELL=/bin/sh\0").unwrap(),
            CStr::from_bytes_with_nul(b"HOME=/root\0").unwrap(),
            CStr::from_bytes_with_nul(b"TERM=xterm\0").unwrap(),
            CStr::from_bytes_with_nul(b"PATH=/bin:/usr/bin:/sbin:/usr/sbin\0").unwrap(),
        ];

        child
            .exec(command_cstr, &argv, &envp)
            .context("Failed to exec")
            .unwrap();

        return 0; // Unreachable but neccessary for clone
    }

    fn setns(child_pid: Pid) -> Result<()> {
        let raw_child_pid = child_pid.as_raw() as isize;

        let userns_filename = format!("/proc/{}/ns/user", raw_child_pid);
        let mntns_filename = format!("/proc/{}/ns/mnt", raw_child_pid);

        let userns = File::open(&userns_filename)
            .with_context(|| format!("Failed to open '{}", userns_filename))?;
        let mntns = File::open(&mntns_filename)
            .with_context(|| format!("Failed to open '{}", mntns_filename))?;

        let userns_fd = userns.as_raw_fd();
        let mntns_fd = mntns.as_raw_fd();

        setns(userns_fd, CloneFlags::CLONE_NEWUSER).context("Failed to setns to userns")?;
        setns(mntns_fd, CloneFlags::CLONE_NEWNS).context("Failed to setns to mntns")?;

        Ok(())
    }
}
