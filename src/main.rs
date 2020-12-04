// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/crca>

extern crate pentry;
extern crate nix;
#[macro_use]
extern crate clap;
extern crate libc;
use nix::sched::*;
use nix::unistd;
use nix::mount;
use nix::sys::wait;
use std::ffi::{CStr, CString};
use clap::{App, Arg};
use libc::SIGCHLD;

fn main() {
    let input = cli();
    let matches = input.get_matches();
    let path = formatter(&matches);

    const STACK_SIZE: usize = 1024 * 1024;
    let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];

    let cb = Box::new(|| child(path));

    let mut clone_flags = CloneFlags::empty();
    clone_flags.insert(CloneFlags::CLONE_NEWUSER);
    clone_flags.insert(CloneFlags::CLONE_NEWUTS);
    clone_flags.insert(CloneFlags::CLONE_NEWIPC);
    clone_flags.insert(CloneFlags::CLONE_NEWPID);
    clone_flags.insert(CloneFlags::CLONE_NEWNET);
    clone_flags.insert(CloneFlags::CLONE_NEWNS);

    let sig_flag_bits: libc::c_int = SIGCHLD;
    let sig_flag: CloneFlags = CloneFlags::from_bits(sig_flag_bits).expect("failed to change SIGCHLD to CloneFlags");
    clone_flags.insert(sig_flag);

    let pid = clone(cb, stack, clone_flags, None);
    match pid {
        Ok(_pid)  => println!("success to clone"),
        Err(_err) => println!("failed to clone"),
    };
    let res = wait::waitpid(pid.unwrap(), Some(wait::WaitPidFlag::WEXITED));
    match res {
        Ok(_ok) => println!("Success to wait"),
        Err(_err) => println!("failed to wait"),
    }
}

fn child(path: &str) -> isize {
    print_process_info();

    mount("proc", "/proc", "proc", "");
    mount("devpts", "/dev/pts", "devpts", "");

    let mut argv: Vec<&CStr> = Vec::new();

    let path_cstring = CString::new(path).expect("failed to CString::new");
    let path_cstr = CStr::from_bytes_with_nul(path_cstring
                                              .to_bytes_with_nul())
                                              .expect("failed to assign to CStr from CString");
    println!("path:{}", path_cstr.to_str().unwrap());
    argv.push(path_cstr);

    let res = unistd::execvp(path_cstr, &argv);
    match res {
        Ok(_ok) => println!("Success to exec"),
        Err(_err) => println!("failed to exec"),
    }

    return 0;
}

fn print_process_info() {
    if let Ok(ps) = pentry::current() {
        println!("{:?}", ps);
    };
}

fn mount(src: &str, trg: &str, fstyp: &str, data: &str) {
    mount::mount(Some(src),
                 trg,
                 Some(fstyp),
                 mount::MsFlags::empty(),
                 Some(data))
                .expect("failed to mount");
}

fn cli() -> App<'static, 'static> {
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
