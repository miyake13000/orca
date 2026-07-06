//! TTY plumbing: SCM_RIGHTS fd passing, raw mode, winsize, and the
//! stdin ↔ PTY-master relay thread.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};

use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::pty::Winsize;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
use nix::sys::termios::{SetArg, Termios, cfmakeraw, tcgetattr, tcsetattr};

use super::ContainerError;

nix::ioctl_read_bad!(tiocgwinsz, libc::TIOCGWINSZ, Winsize);
nix::ioctl_write_ptr_bad!(tiocswinsz, libc::TIOCSWINSZ, Winsize);

fn tty_err(context: &str, e: impl std::fmt::Display) -> ContainerError {
    ContainerError::Tty(format!("{context}: {e}"))
}

/// Send `fd` over the socketpair via `SCM_RIGHTS` (child side).
pub(crate) fn send_fd(sock: RawFd, fd: RawFd) -> nix::Result<()> {
    use nix::sys::socket::{ControlMessage, MsgFlags, sendmsg};
    use std::io::IoSlice;

    let payload = [0u8; 1];
    let iov = [IoSlice::new(&payload)];
    let fds = [fd];
    let cmsg = [ControlMessage::ScmRights(&fds)];
    sendmsg::<()>(sock, &iov, &cmsg, MsgFlags::empty(), None)?;
    Ok(())
}

/// Receive one fd over the socketpair via `SCM_RIGHTS` (parent side).
///
/// Fails with [`ContainerError::InitTimeout`] semantics if the child
/// closed the socket without sending (covered by `await_init` normally;
/// this is a second line of defense).
pub(crate) fn recv_fd(sock: &OwnedFd) -> Result<OwnedFd, ContainerError> {
    use nix::sys::socket::{ControlMessageOwned, MsgFlags, recvmsg};
    use std::io::IoSliceMut;

    let mut payload = [0u8; 1];
    let mut iov = [IoSliceMut::new(&mut payload)];
    let mut cmsg_buf = nix::cmsg_space!([RawFd; 1]);
    let msg = recvmsg::<()>(
        sock.as_raw_fd(),
        &mut iov,
        Some(&mut cmsg_buf),
        MsgFlags::empty(),
    )
    .map_err(|e| tty_err("recvmsg", e))?;
    for cmsg in msg.cmsgs().map_err(|e| tty_err("cmsgs", e))? {
        if let ControlMessageOwned::ScmRights(fds) = cmsg
            && let Some(fd) = fds.first()
        {
            // SAFETY: the kernel just installed this fd for us; we own it.
            return Ok(unsafe { OwnedFd::from_raw_fd(*fd) });
        }
    }
    Err(ContainerError::Tty(
        "child closed the socket without sending a PTY master".to_string(),
    ))
}

/// Write end of the SIGWINCH self-pipe (0 = unset). Written by the signal
/// handler, drained by the relay thread.
static WINCH_PIPE_W: AtomicI32 = AtomicI32::new(0);

/// Async-signal-safe SIGWINCH handler: poke the self-pipe.
extern "C" fn winch_handler(_sig: libc::c_int) {
    let fd = WINCH_PIPE_W.load(Ordering::Relaxed);
    if fd > 0 {
        let byte = [1u8];
        unsafe {
            libc::write(fd, byte.as_ptr() as *const libc::c_void, 1);
        }
    }
}

/// Manages the host tty and the relay thread while a tty container runs:
/// raw mode on the host terminal, initial/forwarded winsize on the PTY
/// master, and a poll loop shuttling bytes between host stdin/stdout and
/// the master.
pub(crate) struct TtyConnector {
    original_termios: Termios,
    handle: Option<std::thread::JoinHandle<()>>,
    shutdown_w: OwnedFd,
    restored: bool,
}

impl TtyConnector {
    /// Take ownership of the received master fd: set the initial winsize,
    /// install the SIGWINCH forwarder, put the host tty into raw mode and
    /// start the relay thread.
    pub(crate) fn start(master: OwnedFd) -> Result<Self, ContainerError> {
        let stdin = std::io::stdin();
        let original_termios =
            tcgetattr(stdin.as_fd()).map_err(|e| tty_err("tcgetattr", e))?;

        // Initial window size: host stdin -> master (propagates to the
        // slave and raises SIGWINCH in the container's foreground group).
        if let Ok(ws) = get_winsize(stdin.as_fd().as_raw_fd()) {
            let _ = set_winsize(master.as_raw_fd(), &ws);
        }

        // SIGWINCH self-pipe + handler.
        let (winch_r, winch_w) = nix::unistd::pipe2(
            nix::fcntl::OFlag::O_CLOEXEC | nix::fcntl::OFlag::O_NONBLOCK,
        )
        .map_err(|e| tty_err("pipe2(winch)", e))?;
        WINCH_PIPE_W.store(winch_w.as_raw_fd(), Ordering::Relaxed);
        let action = SigAction::new(
            SigHandler::Handler(winch_handler),
            SaFlags::SA_RESTART,
            SigSet::empty(),
        );
        // SAFETY: handler only does an atomic load and a write(2).
        unsafe { sigaction(Signal::SIGWINCH, &action) }
            .map_err(|e| tty_err("sigaction(SIGWINCH)", e))?;

        // Shutdown pipe for wait().
        let (shutdown_r, shutdown_w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)
            .map_err(|e| tty_err("pipe2(shutdown)", e))?;

        // Raw mode on the host terminal.
        let mut raw = original_termios.clone();
        cfmakeraw(&mut raw);
        tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &raw)
            .map_err(|e| tty_err("tcsetattr(raw)", e))?;

        let handle = std::thread::spawn(move || {
            relay_loop(master, shutdown_r, winch_r, winch_w);
        });

        Ok(Self {
            original_termios,
            handle: Some(handle),
            shutdown_w,
            restored: false,
        })
    }

    /// Stop the relay (idempotent), join the thread, and restore the host
    /// terminal.
    pub(crate) fn wait(mut self) -> Result<(), ContainerError> {
        let byte = [1u8];
        unsafe {
            libc::write(
                self.shutdown_w.as_raw_fd(),
                byte.as_ptr() as *const libc::c_void,
                1,
            );
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.restore();
        Ok(())
    }

    fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;
        WINCH_PIPE_W.store(0, Ordering::Relaxed);
        let action = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        // SAFETY: restoring SIG_DFL is always safe.
        let _ = unsafe { sigaction(Signal::SIGWINCH, &action) };
        let stdin = std::io::stdin();
        let _ = tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &self.original_termios);
    }
}

impl Drop for TtyConnector {
    /// Restore the terminal even on unwind. Never panics.
    fn drop(&mut self) {
        self.restore();
    }
}

/// The relay: poll host stdin, the PTY master, the shutdown pipe and the
/// SIGWINCH pipe. Exits when the master hangs up (child exited) or a
/// shutdown byte arrives. Read/write errors terminate the loop without
/// panicking.
fn relay_loop(master: OwnedFd, shutdown_r: OwnedFd, winch_r: OwnedFd, _winch_w: OwnedFd) {
    let stdin_fd: RawFd = libc::STDIN_FILENO;
    let stdout_fd: RawFd = libc::STDOUT_FILENO;
    let mut buf = [0u8; 8192];
    let mut stdin_open = true;

    loop {
        // SAFETY: fds are open for the lifetime of this loop.
        let stdin_borrow = unsafe { BorrowedFd::borrow_raw(stdin_fd) };
        let mut fds = vec![
            PollFd::new(master.as_fd(), PollFlags::POLLIN),
            PollFd::new(shutdown_r.as_fd(), PollFlags::POLLIN),
            PollFd::new(winch_r.as_fd(), PollFlags::POLLIN),
        ];
        if stdin_open {
            fds.push(PollFd::new(stdin_borrow, PollFlags::POLLIN));
        }
        match nix::poll::poll(&mut fds, PollTimeout::NONE) {
            Ok(_) => {}
            Err(nix::Error::EINTR) => continue,
            Err(_) => break,
        }
        let master_ev = fds[0].revents().unwrap_or(PollFlags::empty());
        let shutdown_ev = fds[1].revents().unwrap_or(PollFlags::empty());
        let winch_ev = fds[2].revents().unwrap_or(PollFlags::empty());
        let stdin_ev = if stdin_open {
            fds[3].revents().unwrap_or(PollFlags::empty())
        } else {
            PollFlags::empty()
        };

        if shutdown_ev.intersects(PollFlags::POLLIN | PollFlags::POLLHUP) {
            break;
        }
        if winch_ev.contains(PollFlags::POLLIN) {
            let mut drain = [0u8; 64];
            while raw_read(winch_r.as_raw_fd(), &mut drain).unwrap_or(0) > 0 {}
            if let Ok(ws) = get_winsize(stdin_fd) {
                let _ = set_winsize(master.as_raw_fd(), &ws);
            }
        }
        if master_ev.contains(PollFlags::POLLIN) {
            match raw_read(master.as_raw_fd(), &mut buf) {
                Ok(0) | Err(_) => break, // child side closed (EIO on Linux)
                Ok(n) => {
                    if raw_write_all(stdout_fd, &buf[..n]).is_err() {
                        break;
                    }
                }
            }
        } else if master_ev.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
            break;
        }
        if stdin_ev.contains(PollFlags::POLLIN) {
            match raw_read(stdin_fd, &mut buf) {
                Ok(0) | Err(_) => stdin_open = false,
                Ok(n) => {
                    if raw_write_all(master.as_raw_fd(), &buf[..n]).is_err() {
                        break;
                    }
                }
            }
        } else if stdin_ev.intersects(PollFlags::POLLHUP | PollFlags::POLLERR) {
            stdin_open = false;
        }
    }
}

fn raw_read(fd: RawFd, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EAGAIN) => return Ok(0),
            _ => return Err(err),
        }
    }
}

fn raw_write_all(fd: RawFd, mut buf: &[u8]) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(err);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

fn get_winsize(fd: RawFd) -> nix::Result<Winsize> {
    let mut ws = Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: standard TIOCGWINSZ on a tty fd.
    unsafe { tiocgwinsz(fd, &mut ws) }?;
    Ok(ws)
}

fn set_winsize(fd: RawFd, ws: &Winsize) -> nix::Result<()> {
    // SAFETY: standard TIOCSWINSZ on the PTY master.
    unsafe { tiocswinsz(fd, ws) }?;
    Ok(())
}
