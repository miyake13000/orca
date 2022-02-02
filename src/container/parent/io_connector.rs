use anyhow::Result;
use nix::unistd::{read, write};
use std::os::unix::io::RawFd;
use std::thread;

pub struct IoConnector;

impl IoConnector {
    pub fn new(
        parent_stdout_fd: RawFd,
        parent_stdin_fd: RawFd,
        child_stdout_fd: RawFd,
        child_stdin_fd: RawFd,
    ) -> Self {
        thread::spawn(move || {
            let mut s: [u8; 1] = [0; 1];
            loop {
                if read(child_stdout_fd, &mut s).is_err() {
                    return;
                };
                if write(parent_stdout_fd, &s).is_err() {
                    return;
                };
            }
        });

        thread::spawn(move || {
            let mut s: [u8; 1] = [0; 1];
            loop {
                if read(parent_stdin_fd, &mut s).is_err() {
                    return;
                }
                if write(child_stdin_fd, &s).is_err() {
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
