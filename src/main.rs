// This program is created by nomlab in Okayama University
// https://github.com/miyake13000/crca

//use fork::{daemon, Fork};
//use std::process::{Command, Child, Stdio};
use std::process::Command;
//use std::os::unix::io::{AsRawFd, FromRawFd};

fn main() {
    println!("starting fork...");
    let output = Command::new("sleep")
        .arg("10")
        .output()
        .expect("faisled to execuse process");
    println!("done.");
    println!("status: {}", output.status);
    println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
}
