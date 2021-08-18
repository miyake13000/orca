use std::io::stdin;
use std::os::unix::io::{AsRawFd, RawFd};
use nix::sys::termios::{Termios, tcgetattr, tcsetattr, cfmakeraw, SetArg};

pub struct Terminal {
    terminal_fd: RawFd,
    current_termios: Termios,
    orig_termios: Termios,
}

impl Terminal {
    pub fn new() -> Self {
        let terminal_fd = stdin().as_raw_fd();
        let current_termios = tcgetattr(terminal_fd).unwrap();
        let orig_termios = current_termios.clone();

        Terminal{
            terminal_fd,
            current_termios,
            orig_termios
        }
    }

    pub fn into_raw_mode(&mut self) -> std::result::Result<(), ()> {
        cfmakeraw(&mut self.current_termios);
        tcsetattr(self.terminal_fd, SetArg::TCSAFLUSH, &self.current_termios).unwrap();

        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        tcsetattr(self.terminal_fd, SetArg::TCSAFLUSH, &self.orig_termios).unwrap();
    }
}
