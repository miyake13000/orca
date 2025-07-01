use anyhow::Result;
use nix::unistd::{read, write};
use std::os::fd::OwnedFd;
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
            loop {
                if read(&child_stdout, &mut s).is_err() {
                    return;
                };
                if write(&parent_stdout, &s).is_err() {
                    return;
                };
            }
        });

        thread::spawn(move || {
            let mut s: [u8; 1] = [0; 1];
            loop {
                if read(&parent_stdin, &mut s).is_err() {
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
