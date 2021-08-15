mod child;
mod id;

use std::ffi::{CString, CStr};
use std::process::Command;
use nix::unistd::Pid;
use nix::sys::wait::wait;
use nix::sched::{clone, CloneFlags};
use child::Child;
use id::{MappingId, IdType};
use crate::image::Image;
use crate::STACK_SIZE;

pub struct Container {
    image: Image,
    child_pid: Pid,
}

impl Container {
    pub fn new(
        image: Image,
        command: String,
        netns_flag: bool
    ) -> Self {

        let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];

        let cb = Box::new(|| Self::child_main(&command, &image.rootfs_path, &image.image_name));

        let mut flags = CloneFlags::empty();
        flags.insert(CloneFlags::CLONE_NEWUSER);
        flags.insert(CloneFlags::CLONE_NEWNS);
        flags.insert(CloneFlags::CLONE_NEWUTS);
        flags.insert(CloneFlags::CLONE_NEWPID);
        flags.insert(CloneFlags::CLONE_NEWIPC);
        if netns_flag {
            flags.insert(CloneFlags::CLONE_NEWNET);
        }
        let signals = Some(libc::SIGCHLD);

        let child_pid = clone(cb, stack, flags, signals).unwrap();

        Container{
            image,
            child_pid
        }
    }

    pub fn map_id(&self, subid_flag: bool) -> std::result::Result<(), ()>{
        let mut args_uidmap: Vec<String> = vec![self.child_pid.to_string()];
        let mut args_gidmap: Vec<String> = vec![self.child_pid.to_string()];

        let mapping_uid = MappingId::new(IdType::UID);
        let mapping_gid = MappingId::new(IdType::GID);
        let _ = args_uidmap.append(&mut mapping_uid.into_vec());
        let _ = args_gidmap.append(&mut mapping_gid.into_vec());

        if subid_flag {
            let mapping_subuid = MappingId::new(IdType::SUBUID);
            let mapping_subgid = MappingId::new(IdType::SUBGID);
            let _ = args_uidmap.append(&mut mapping_subuid.into_vec());
            let _ = args_gidmap.append(&mut mapping_subgid.into_vec());
        }

        let _ = Command::new("newuidmap")
            .args(&args_uidmap)
            .output()
            .unwrap();
        let _ = Command::new("newgidmap")
            .args(&args_gidmap)
            .output()
            .unwrap();

        Ok(())
    }

    pub fn wait(self) -> std::result::Result<Image, ()> {
        let _ = wait().unwrap();
        Ok(self.image)
    }

    fn child_main(command: &str, path_rootfs: &str, image_name: &str) -> isize {

        let child = Child::new(path_rootfs.to_string());
        child.pivot_root().unwrap();
        child.mount().unwrap();
        child.sethostname(image_name).unwrap();

        let command_cstring = CString::new(command).unwrap();
        let command_cstr = command_cstring.as_c_str();

        let mut argv: Vec<&CStr> = Vec::new();
        argv.push(command_cstr);

        let mut envp: Vec<&CStr> = Vec::new();
        envp.push(CStr::from_bytes_with_nul(b"SHELL=/bin/sh\0").unwrap());
        envp.push(CStr::from_bytes_with_nul(b"HOME=/root\0").unwrap());
        envp.push(CStr::from_bytes_with_nul(b"TERM=xterm\0").unwrap());
        envp.push(CStr::from_bytes_with_nul(b"PATH=/bin:/usr/bin:/sbin:/usr/sbin\0").unwrap());

        child.exec(command_cstr, &argv, &envp);

        return 0; // Unreachable but neccessary for clone
    }
}
