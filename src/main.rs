// orca : CLI Container Runtime
// This program is created by nomlab in Okayama University
// author nomlab <https://github.com/nomlab>
//        miyake13000 <https://github.com/miyake13000/crca>

#[macro_use]
extern crate clap;
extern crate nix;
extern crate libc;
use nix::sched;
use nix::unistd;
use nix::mount;
use nix::sys::wait;
use std::ffi::{CStr, CString};
use clap::{App, Arg};

fn main() {
    let input = cli();
    let matches = input.get_matches();
    let path = formatter(&matches);

    const STACK_SIZE: usize = 1024 * 1024;
    let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];
    let cb = Box::new(|| child(path));

    clone(cb, stack).expect("failed to clone");
    wait::wait().expect("failed to wait child process");
}

fn child(path: &str) -> isize {
    mount("proc", "/proc", "proc", "");
    mount("devpts", "/dev/pts", "devpts", "");

    let mut argv: Vec<&CStr> = Vec::new();

    let path_cstring = CString::new(path).expect("failed to CString::new");
    let path_cstr = CStr::from_bytes_with_nul(path_cstring
                                              .to_bytes_with_nul())
                                              .expect("failed to assign to CStr from CString");
    argv.push(path_cstr);
    unistd::execvp(path_cstr, &argv).expect("failed to execvp");

    return 0;
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
