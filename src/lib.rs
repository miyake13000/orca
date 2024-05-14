pub mod args;
pub mod container;
pub mod image;
pub mod mount;
pub mod vcs;

const STACK_SIZE: usize = 1024 * 1024;

use anyhow::{Error, Result};
use std::process::exit;

trait OrExit<T> {
    fn or_exit(self) -> T;
}

impl<T> OrExit<T> for Result<T> {
    fn or_exit(self) -> T {
        match self {
            Ok(res) => res,
            Err(e) => {
                print_error(e);
                exit(1);
            }
        }
    }
}

fn print_error(err: Error) {
    eprintln!("Error (child process): {}\n", err);
    eprintln!("Caused by:");
    err.chain()
        .skip(1)
        .for_each(|cause| eprintln!("    {}", cause));
}
