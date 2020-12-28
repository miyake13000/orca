// This file has wrap function of syscall

extern crate libc;
extern crate nix;

use nix::{sched, unistd, mount, sys};
use std::ffi::CStr;

pub fn clone(cb: sched::CloneCb,
             stack: &mut [u8],
             user_flag: bool,
             mnt_flag: bool,
             uts_flag: bool,
             pid_flag: bool,
             ipc_flag: bool,
             net_flag: bool)
    -> std::result::Result<i32, ()> {

    let mut flags = sched::CloneFlags::empty();

    if user_flag {
        flags.insert(sched::CloneFlags::CLONE_NEWUSER);
    }
    if mnt_flag {
        flags.insert(sched::CloneFlags::CLONE_NEWNS);
    }
    if uts_flag {
        flags.insert(sched::CloneFlags::CLONE_NEWUTS);
    }
    if pid_flag {
        flags.insert(sched::CloneFlags::CLONE_NEWPID);
    }
    if ipc_flag {
        flags.insert(sched::CloneFlags::CLONE_NEWIPC);
    }
    if net_flag {
        flags.insert(sched::CloneFlags::CLONE_NEWNET);
    }

    let signals = Some(libc::SIGCHLD);

    // Execute clone syscall
    let res = sched::clone(cb, stack, flags, signals);

    match res {
        Ok(pid) => Ok(pid.as_raw() as i32),
        Err(_)  => Err(()),
    }
}

pub fn mount(src: &str, trg: &str, fstype: &str, bind_flag: bool) -> std::result::Result<(),()>{
    let flag = if bind_flag {
        mount::MsFlags::MS_BIND
    }else{
        mount::MsFlags::empty()
    };

    // Execute mount syscall
    let res = mount::mount(Some(src), trg, Some(fstype), flag, Some(""));

    match res {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn umount(trg: &str, detach_flag: bool) -> std::result::Result<(), ()>{
    let flag = if detach_flag {
        mount::MntFlags::MNT_DETACH
    }else{
        mount::MntFlags::empty()
    };

    // Execute umount syscall
    let res = mount::umount2(trg, flag);

    match res {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn wait() -> std::result::Result<(),()>{
    // Execute wait syscall
    let res = sys::wait::wait();

    match res {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn chdir(path: &str) -> std::result::Result<(), ()>{
    // Execute chdir syscall
    let res = unistd::chdir(path);

    match res {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn pivot_root(path: &str, oldroot: &str) -> std::result::Result<(), ()>{
    // Execute pivot_root syscall
    let res = unistd::pivot_root(path, oldroot);

    match res {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn sethostname(name: &str) -> std::result::Result<(), ()>{
    // Execute sethostname syscall
    let res = unistd::sethostname(name);

    match res {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

pub fn execvpe(filename: &CStr, args: &[&CStr], env: &[&CStr]) -> std::result::Result<(), ()>{
    // Execute execvpe syscall
    let res = unistd::execvpe(filename, args, env);

    match res {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}
