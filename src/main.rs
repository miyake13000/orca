// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/orca>

#[macro_use]
extern crate clap;
extern crate nix;
extern crate libc;

use nix::{sched, unistd, mount, sys};
use std::ffi::{CStr, CString};
use std::io::{Write, Result};
use std::fs::{File, create_dir_all};
use clap::{App, Arg};
use dirs::home_dir;

mod image;

fn main() {
    let input = get_input();
    let matches = input.get_matches();
    let command = formatter(&matches);

    let dest_name = String::from("debian");
    let dest_tag = String::from("latest");
    let home_dir = home_dir().unwrap();
    let home_dir_str = home_dir.to_str().unwrap();
    let path = format!("{}/.local/orca/containers/{}/{}", home_dir_str, dest_name, dest_tag);
    let image = image::Image::new(dest_name, dest_tag, path);
    if  !image.exist_image() {
        println!("Cannot find container image on local");
        image.create_dir().unwrap();
        println!("Downloading container image...");
        image.get().unwrap();
    }
    if !image.exist_rootfs() {
        println!("Extracting container image...");
        image.extract().unwrap();
    }

    const STACK_SIZE: usize = 1024 * 1024;
    let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];
    let cb = Box::new(|| child(command, &image));

    let pid = clone(cb, stack).expect("clone");
    let pid_int = pid.as_raw() as i32;
    let uid = unistd::getuid().as_raw() as u32;
    id_map(pid_int, 0, uid, 1).expect("set_uid");
    sys::wait::wait().expect("wait");
}

fn child(command: &str, image: &image::Image) -> isize {
    unistd::chdir(image.path_rootfs.as_path()).expect("chdir");
    unistd::chroot(image.path_rootfs.as_path()).expect("chroot");
    unistd::sethostname(&image.dest_name).expect("sethostname");

    create_dir_all("/proc").unwrap();
    mount("proc", "/proc", "proc", "").expect("mount proc");
    create_dir_all("/dev/pts").unwrap();
    mount("devpts", "/dev/pts", "devpts", "").expect("mount devpts");

    let mut argv: Vec<&CStr> = Vec::new();

    let command_cstring = CString::new(command).expect("CString::new");
    let command_cstr = CStr::from_bytes_with_nul(command_cstring
                                              .to_bytes_with_nul())
                                              .expect("CString to CStr");
    argv.push(command_cstr);

    let mut envp: Vec<&CStr> = Vec::new();
    envp.push(CStr::from_bytes_with_nul(b"SHELL=/bin/bash\0").expect("env shell"));
    envp.push(CStr::from_bytes_with_nul(b"HOME=/root\0").expect("env home"));
    envp.push(CStr::from_bytes_with_nul(b"TERM=xterm-256color\0").expect("env term"));

    unistd::execvpe(command_cstr, &argv, &envp).expect("execvpe");

    return 0;
}

fn id_map(pid: i32, innner_id: u32, outer_id: u32, lenge: u32) -> Result<usize> {
    let path = format!("{}{}", "/proc/", pid.to_string());
    let path_uid = format!("{}{}", path, "/uid_map");
    let path_gid = format!("{}{}", path, "/gid_map");
    let path_setg = format!("{}{}", path, "/setgroups");
    let content = format!("{} {} {}", innner_id, outer_id, lenge);
    let mut file_uid = File::create(path_uid).unwrap();
    let mut file_gid = File::create(path_gid).unwrap();
    let mut file_setg = File::create(path_setg).unwrap();
    file_uid.write(content.as_bytes()).expect("write uid");
    file_setg.write(b"deny").expect("write setg");
    file_gid.write(content.as_bytes())
}

fn clone(cb: sched::CloneCb, stack: &mut [u8]) -> nix::Result<unistd::Pid> {

    let mut flags = sched::CloneFlags::empty();
    flags.insert(sched::CloneFlags::CLONE_NEWUSER);
    flags.insert(sched::CloneFlags::CLONE_NEWUTS);
    flags.insert(sched::CloneFlags::CLONE_NEWIPC);
    flags.insert(sched::CloneFlags::CLONE_NEWPID);
    flags.insert(sched::CloneFlags::CLONE_NEWNET);
    flags.insert(sched::CloneFlags::CLONE_NEWNS);

    let signal = Some(libc::SIGCHLD);

    sched::clone(cb, stack, flags, signal)
}

fn mount(src: &str, trg: &str, fstyp: &str, data: &str) -> nix::Result<()> {
    mount::mount(Some(src),
                 trg,
                 Some(fstyp),
                 mount::MsFlags::empty(),
                 Some(data))
}

fn get_input() -> App<'static, 'static> {
    let app = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("command")
             .help("command to execute in conainer")
             .required(false)
            );
    return app
}

fn formatter<'a>(matches: &'a clap::ArgMatches) -> &'a str {
    if let Some(o) = matches.value_of("command") {
        o
    }else {
        "/bin/bash"
    }
}

