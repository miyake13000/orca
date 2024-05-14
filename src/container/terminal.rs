use anyhow::{anyhow, Context, Result};
use nix::libc::{TIOCGWINSZ, TIOCSWINSZ};
use nix::pty::Winsize;
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg, Termios};
use nix::{ioctl_read_bad, ioctl_readwrite_bad};
use std::os::fd::{AsFd, AsRawFd};

pub struct Terminal {
    terminal: Box<dyn AsFd>,
    current_termios: Termios,
    orig_termios: Termios,
}

impl Terminal {
    pub fn new<R: AsFd + 'static>(terminal: R) -> Result<Self> {
        let current_termios =
            tcgetattr(&terminal).context("Failed to get current terminal settings")?;
        let orig_termios = current_termios.clone();

        Ok(Terminal {
            terminal: Box::new(terminal),
            current_termios,
            orig_termios,
        })
    }

    pub fn make_raw_mode(&mut self) -> Result<()> {
        cfmakeraw(&mut self.current_termios);
        tcsetattr(&self.terminal, SetArg::TCSAFLUSH, &self.current_termios)
            .context("Failed to change terminal settings")?;

        Ok(())
    }

    pub fn get_win_size(&self) -> Result<Winsize> {
        let mut win_size = Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let res = unsafe { get_winsize(self.terminal.as_fd().as_raw_fd(), &mut win_size) };
        match res {
            Ok(_) => Ok(win_size),
            Err(_) => Err(anyhow!("Failed to get window size")),
        }
    }

    pub fn set_win_size(&mut self, win_size: &mut Winsize) -> Result<()> {
        let res = unsafe { set_winsize(self.terminal.as_fd().as_raw_fd(), win_size) };
        match res {
            Ok(_) => Ok(()),
            Err(_) => Err(anyhow!("Failed to change window size")),
        }
    }
}

ioctl_read_bad!(get_winsize, TIOCGWINSZ, Winsize);
ioctl_readwrite_bad!(set_winsize, TIOCSWINSZ, Winsize);

impl Drop for Terminal {
    fn drop(&mut self) {
        tcsetattr(&self.terminal, SetArg::TCSAFLUSH, &self.orig_termios)
            .context("Failed to reverse terminal settings")
            .unwrap();
    }
}
