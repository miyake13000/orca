use anyhow::Result;
use nix::unistd::{read, write};
use std::os::fd::{AsFd, OwnedFd};
use std::thread;

pub struct IoConnector;

impl IoConnector {
    pub fn new(
        parent_stdout: OwnedFd,
        parent_stdin: OwnedFd,
        child_stdout: OwnedFd,
        child_stdin: OwnedFd,
    ) -> Self {
        thread::spawn(move || {
            let mut s: [u8; 1] = [0; 1];
            let child_stdout_fd = child_stdout.as_fd();
            loop {
                if read(child_stdout_fd, &mut s).is_err() {
                    return;
                };
                if write(&parent_stdout, &s).is_err() {
                    return;
                };
            }
        });

        thread::spawn(move || {
            let mut s: [u8; 1] = [0; 1];
            let parent_stdin_fd = parent_stdin.as_fd();
            loop {
                if read(parent_stdin_fd, &mut s).is_err() {
                    return;
                }
                if write(&child_stdin, &s).is_err() {
                    return;
                }
            }
        });
        Self {}
    }

    pub fn stop(self) -> Result<()> {
        //TODO: add process to join thread
        Ok(())
    }
}
