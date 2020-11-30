// This program is created by nomlab in Okayama University
// https://github.com/miyake13000/crca
extern crate pentry;
extern crate nix;
use nix::sched::*;
//use std::process::Command;
//use std::process::{Command, Child, Stdio};

fn main() {
    print_process_info();
    const STACK_SIZE: usize = 1024 * 1024;
    let ref mut stack: [u8; STACK_SIZE] = [0; STACK_SIZE];

    let cb = Box::new(|| child());

    let clone_flags: CloneFlags = CloneFlags::CLONE_NEWUTS ||
                                  CloneFlags::CLONE_NEWIPC ||
                                  CloneFlags::CLONE_NEWPID ||
                                  CloneFlags::CLONE_NEWNET ||
                                  CloneFlags::CLONE_NEWNS;

    let p = clone(cb, stack, clone_flags, None);

    let p = match p {
        Ok(p) => p,
        Err(err) => panic!(err),
    };
}

fn child() -> isize {
    print_process_info();
    return 0;
}

fn print_process_info() {
    if let Ok(ps) = pentry::current() {
        println!("{:?}", ps);
    };
}
