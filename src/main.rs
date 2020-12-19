// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/orca>

#[macro_use]
extern crate clap;
extern crate nix;
extern crate libc;
extern crate dirs;

use std::ffi::{CStr, CString};
use std::io;
use std::fs;
use std::path::Path;
use std::process::Command;
use clap::{App, Arg, ArgMatches};
use nix::{sched, unistd, mount, sys};

mod image;

struct Input<'a, 'b, 'c> {
    name: &'a str,
    tag: &'b str,
    command: &'c str,
    init_flag: bool,
    remove_flag: bool,
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
    } else {
        // init setting for container image
        if input.init_flag {
            fs::remove_dir_all(&path_rootfs).unwrap();
            println!("Extracting...");
            fs::create_dir_all(&path_rootfs).unwrap();
            image.extract(&path_image, &path_rootfs).unwrap();
        }
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

    // terminating process for caontainer image
    if input.remove_flag {
        fs::remove_dir_all(&path_rootfs).unwrap();
    }
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

fn id_map(pid: i32, inner_id: u32, outer_id: u32, range: u32) -> io::Result<()> {
    let lowest_subid = (outer_id - 1000) * 65536 + 100000;
    let args: Vec<String> = vec![pid.to_string(),
                                 inner_id.to_string(),
                                 outer_id.to_string(),
                                 range.to_string(),
                                 "1".to_string(),
                                 lowest_subid.to_string(),
                                 "65536".to_string()
                                ];
    let _ = Command::new("newuidmap")
        .args(&args)
        .output()
        .expect("id_map");

    let _ = Command::new("newgidmap")
        .args(&args)
        .output()
        .expect("id_map");

    Ok(())
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
        .arg(Arg::with_name("init")
             .short("i")
             .long("init")
             .help("initialize contaier environment")
            )
        .arg(Arg::with_name("remove")
             .short("r")
             .long("remove")
             .help("remove container environment after executing")
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
        command: command,
        init_flag: matches.is_present("init"),
        remove_flag: matches.is_present("remove")
    }
}

#[test]
fn bench_image() {
    use std::time::Instant;

    println!("start bench");

    let name = "debian";
    let tag  = "latest";
    let home_dir = dirs::home_dir().unwrap();
    let home_dir_str = home_dir.to_str().unwrap();
    let path = format!("{}/.local/orca/containers/{}/{}", home_dir_str, name, tag);
    let path_image = format!("{}/image.tar.gz", path);
    let path_rootfs = format!("{}/rootfs", path);
    let image = image::Image::new(name, tag);

    let start = Instant::now();

    println!("start get_token");
    let token = image.get_token().unwrap();
    let res1 = start.elapsed().as_millis();
    println!("end get_token {}ms", res1);

    println!("start get_leyaer_id");
    let layer_id = image.get_layer_id(&token).unwrap();
    let res2 = start.elapsed().as_millis();
    println!("end get_layer_id {}ms", res2 - res1);

    fs::create_dir_all(&path).unwrap();

    println!("start download");
    image.download(&token, &layer_id, &path_image).unwrap();
    let res3 = start.elapsed().as_millis();
    println!("end download {}ms", res3- res2);

    println!("start extract");
    fs::create_dir_all(&path_rootfs).unwrap();
    image.extract(&path_image, &path_rootfs).unwrap();
    let res4 = start.elapsed().as_millis();
    println!("end extract {}ms", res4 - res3);

    fs::remove_file(path_image).unwrap();
    fs::remove_dir_all(path_rootfs).unwrap();

    println!("end bench");
}

