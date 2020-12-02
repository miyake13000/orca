// This program is created by nomlab in Okayama University
// https://github.com/miyake13000/crca

extern crate pentry;
extern crate nix;
use nix::sched::*;
use nix::unistd;
use std::ffi::CStr;

fn main() {
    print_process_info();

    const STACK_SIZE: usize = 1024 * 1024;
    let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];
    let cb = Box::new(|| child());

    let mut clone_flags = CloneFlags::empty();
    clone_flags.insert(CloneFlags::CLONE_NEWUSER);
    clone_flags.insert(CloneFlags::CLONE_NEWUTS);
    clone_flags.insert(CloneFlags::CLONE_NEWIPC);
    clone_flags.insert(CloneFlags::CLONE_NEWPID);
    clone_flags.insert(CloneFlags::CLONE_NEWNET);
    clone_flags.insert(CloneFlags::CLONE_NEWNS);

    let p = clone(cb, stack, clone_flags, None);
    match p {
        Ok(_pid)  => println!("success to clone"),
        Err(_err) => println!("failes to clone process"),
    };
}

fn child() -> isize {
    print_process_info();

    let mut argv: Vec<&CStr> = Vec::new();
    let path = CStr::from_bytes_with_nul(b"/bin/ls\0").unwrap();
    argv.push(path);
    let arg = CStr::from_bytes_with_nul(b"-la\0").unwrap();
    argv.push(arg);

    let res = unistd::execv(path, &argv);
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
