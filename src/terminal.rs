use anyhow::{Context, Result};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg, Termios};
use std::io::stdin;
use std::os::unix::io::{AsRawFd, RawFd};

pub struct Terminal {
    terminal_fd: RawFd,
    current_termios: Termios,
    orig_termios: Termios,
}

impl Terminal {
    pub fn new() -> Result<Self> {
        let terminal_fd = stdin().as_raw_fd();
        let current_termios =
            tcgetattr(terminal_fd).context("Failed to get current terminal settings")?;
        let orig_termios = current_termios.clone();

        Ok(Terminal {
            terminal_fd,
            current_termios,
            orig_termios,
        })
    }

    pub fn into_raw_mode(&mut self) -> Result<()> {
        cfmakeraw(&mut self.current_termios);
        tcsetattr(self.terminal_fd, SetArg::TCSAFLUSH, &self.current_termios)
            .context("Failed to change terminal settings")?;

        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        tcsetattr(self.terminal_fd, SetArg::TCSAFLUSH, &self.orig_termios)
            .context("Failed to reverse terminal settings")
            .unwrap();
    }
}
