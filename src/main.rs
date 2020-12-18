// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/orca>

#[macro_use]
extern crate clap;
extern crate nix;
extern crate libc;
extern crate dirs;

use nix::{sched, unistd, mount, sys};
use std::ffi::{CStr, CString};
use std::io::{self, Write};
use std::fs::{self, File};
use std::path::Path;
use clap::{App, Arg, ArgMatches};

mod image;

struct Input<'a, 'b, 'c> {
    name: &'a str,
    tag: &'b str,
    command: &'c str,
}

fn main() {
    // constants in main function
    // TODO : Modify variables to work when changed to constants
    let name = "debian";
    let tag = "latest";
    let command = "sh";

    // get and parse input
    let input_app = get_input();
    let matches = input_app.get_matches();
    let input = formatter(&matches, name, tag, command);

    // variables in main function
    let home_dir = dirs::home_dir().unwrap();
    let home_dir_str = home_dir.to_str().unwrap();
    let path = format!("{}/.local/orca/containers/{}/{}", home_dir_str, input.name, input.tag);
    let path_image = format!("{}/image.tar.gz", path);
    let path_rootfs = format!("{}/rootfs", path);
    let image = image::Image::new(input.name, input.tag);

    // download container image if it doesnt exist
    if !Path::new(&path_image).exists() {
        println!("Cannot find container image on local");
        println!("Serching...");
        let token = image.get_token().unwrap();
        println!("Downloading...");
        let layer_id = image.get_layer_id(&token).unwrap();
        fs::create_dir_all(&path).unwrap();
        image.download(&token, &layer_id, &path_image).unwrap();
    }

    // extract container image if it doesnt exist
    if !Path::new(&path_rootfs).exists() {
        println!("Extracting...");
        fs::create_dir_all(&path_rootfs).unwrap();
        image.extract(&path_image, &path_rootfs).unwrap();
    }

    // variable for child process
    const STACK_SIZE: usize = 1024 * 1024;
    let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];
    let cb = Box::new(|| child(input.command, &path_rootfs, image.dest_name));

    // create child process
    let pid = clone(cb, stack).unwrap();

    // map user's uid and gid to root in container
    let pid = pid.as_raw() as i32;
    let uid = unistd::getuid().as_raw() as u32;
    id_map(pid, 0, uid, 1).expect("set_uid");

    // wait for child process exiting
    sys::wait::wait().expect("wait");
}

fn child(command: &str, path_rootfs: &str, dest_name: &str) -> isize {

    let path_oldroot = format!("{}/oldroot", path_rootfs);
    let path_oldroot = path_oldroot.as_str();

    unistd::chdir(path_rootfs).expect("chdir");
    mount(path_rootfs , path_rootfs, "", mount::MsFlags::MS_BIND, "").expect("mount bind");
    fs::create_dir_all(path_oldroot).expect("create dir oldroot");
    unistd::pivot_root(path_rootfs, path_oldroot).expect("pivot_root");
    unistd::chdir("/").expect("chdir");
    fs::create_dir_all("/proc").expect("crate dir proc");
    mount("proc", "/proc", "proc",mount::MsFlags::empty(), "").expect("mount proc");
    fs::create_dir_all("/dev/pts").expect("create dir devpts");
    mount("devpts", "/dev/pts", "devpts",mount::MsFlags::empty(), "").expect("mount devpts");
    mount::umount2("/oldroot", mount::MntFlags::MNT_DETACH).expect("umount oldroot");
    fs::remove_dir("/oldroot").expect("remove dir oldroot");
    unistd::sethostname(dest_name).expect("sethostname");

    let mut argv: Vec<&CStr> = Vec::new();

    let command_cstring = CString::new(command).expect("CString::new");
    let command_cstr = CStr::from_bytes_with_nul(command_cstring
                                              .to_bytes_with_nul())
                                              .expect("CString to CStr");
    argv.push(command_cstr);

    let mut envp: Vec<&CStr> = Vec::new();
    envp.push(CStr::from_bytes_with_nul(b"SHELL=/bin/sh\0").unwrap());
    envp.push(CStr::from_bytes_with_nul(b"HOME=/root\0").unwrap());
    envp.push(CStr::from_bytes_with_nul(b"TERM=xterm\0").unwrap());
    envp.push(CStr::from_bytes_with_nul(b"PATH=/bin:/usr/bin:/sbin:/usr/sbin\0").unwrap());

    unistd::execvpe(command_cstr, &argv, &envp).expect("execvpe");

    return 0;
}

fn id_map(pid: i32, innner_id: u32, outer_id: u32, lenge: u32) -> io::Result<usize> {
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

fn mount(src: &str, trg: &str, fstyp: &str, flag: mount::MsFlags, data: &str) -> nix::Result<()> {
    mount::mount(Some(src),
                 trg,
                 Some(fstyp),
                 flag,
                 Some(data))
}

fn get_input() -> App<'static, 'static> {
    let app = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("name")
             .short("n")
             .long("name")
             .help("name of container image")
             .takes_value(true)
        )
        .arg(Arg::with_name("tag")
             .short("t")
             .long("tag")
             .help("tag of container iamge")
             .takes_value(true)
        )
        .arg(Arg::with_name("command")
             .help("command to execute in conainer")
            );
    return app
}

fn formatter<'a>(matches: &'a ArgMatches, default_name: &'a str, default_tag: &'a str, default_command: &'a str) -> Input<'a, 'a, 'a> {
    let name = if let Some(o) = matches.value_of("name") {
        o
    }else {
        default_name
    };
    let tag = if let Some(o) = matches.value_of("tag") {
        o
    }else {
        default_tag
    };
    let command = if let Some(o) = matches.value_of("command") {
        o
    }else {
        default_command
    };
    Input {
        name: name,
        tag: tag,
        command: command
    }
}

