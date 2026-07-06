//! Parent-side plumbing: init handshake, waitpid loop and signal
//! forwarding.
//!
//! The parent never enters the child's namespaces (no `setns`); its only
//! channels are the status pipe, the fd-passing socket, and `waitpid`.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};

use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::Pid;

use super::{ContainerError, syscall};

/// How long the parent waits for the child's init handshake.
const INIT_TIMEOUT_MS: u16 = 30_000;

/// Read the status pipe until EOF (success) or data (failed stage).
///
/// EOF means the CLOEXEC write end vanished at the child's `exec`; any
/// bytes are a structured `stage: detail` report from `child::fail`.
pub(crate) fn await_init(status_r: &OwnedFd) -> Result<(), ContainerError> {
    use nix::poll::{PollFd, PollFlags, PollTimeout};

    let mut report = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let mut fds = [PollFd::new(status_r.as_fd_ref(), PollFlags::POLLIN)];
        let n = match nix::poll::poll(&mut fds, PollTimeout::from(INIT_TIMEOUT_MS)) {
            Ok(n) => n,
            Err(nix::Error::EINTR) => continue,
            Err(e) => return Err(syscall("poll(status)")(e)),
        };
        if n == 0 {
            return Err(ContainerError::InitTimeout);
        }
        let read = unsafe {
            libc::read(
                status_r.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        match read {
            0 => break, // EOF
            n if n < 0 => {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(ContainerError::Syscall {
                    op: "read(status)",
                    source: nix::Error::from_raw(err.raw_os_error().unwrap_or(0)),
                });
            }
            n => report.extend_from_slice(&buf[..n as usize]),
        }
    }
    if report.is_empty() {
        Ok(())
    } else {
        Err(ContainerError::ChildInit(
            String::from_utf8_lossy(&report).into_owned(),
        ))
    }
}

/// Extension to get a `BorrowedFd` out of an `OwnedFd` reference (helper
/// for `PollFd::new`).
trait AsFdRef {
    fn as_fd_ref(&self) -> std::os::fd::BorrowedFd<'_>;
}

impl AsFdRef for OwnedFd {
    fn as_fd_ref(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.as_fd()
    }
}

/// Loop `waitpid` until the child exits or is killed, returning the exit
/// code (`WIFEXITED` code, or `128 + signo` when signaled).
pub(crate) fn wait_child(pid: Pid) -> Result<i32, ContainerError> {
    loop {
        match waitpid(pid, None) {
            Ok(WaitStatus::Exited(_, code)) => return Ok(code),
            Ok(WaitStatus::Signaled(_, sig, _)) => return Ok(128 + sig as i32),
            Ok(_) => continue, // stopped/continued etc.
            Err(nix::Error::EINTR) => continue,
            Err(e) => return Err(syscall("waitpid")(e)),
        }
    }
}

/// Kill (best-effort) and reap a child after an init failure so no zombie
/// outlives the error path.
pub(crate) fn reap(pid: Pid) -> Result<i32, ContainerError> {
    let _ = nix::sys::signal::kill(pid, Signal::SIGKILL);
    wait_child(pid)
}

/// Child PID that the forwarding signal handler targets (0 = none).
static FORWARD_PID: AtomicI32 = AtomicI32::new(0);

/// Async-signal-safe handler: forward the signal to the child. `kill(2)`
/// is on the async-signal-safe list.
extern "C" fn forward_handler(sig: libc::c_int) {
    let pid = FORWARD_PID.load(Ordering::Relaxed);
    if pid > 0 {
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

/// Install SIGINT/SIGTERM/SIGHUP forwarding to the child (`IoMode::Piped`
/// only; in tty mode Ctrl-C travels through the PTY instead). With a PID
/// namespace, killing the child (PID 1) tears down the whole namespace.
pub(crate) fn forward_signals(child: Pid) -> Result<(), ContainerError> {
    FORWARD_PID.store(child.as_raw(), Ordering::Relaxed);
    let action = SigAction::new(
        SigHandler::Handler(forward_handler),
        SaFlags::SA_RESTART,
        SigSet::empty(),
    );
    for sig in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
        // SAFETY: handler is async-signal-safe (atomic load + kill).
        unsafe { sigaction(sig, &action) }.map_err(syscall("sigaction"))?;
    }
    Ok(())
}

/// Restore default dispositions after the child exited.
pub(crate) fn stop_forwarding() {
    FORWARD_PID.store(0, Ordering::Relaxed);
    let action = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
    for sig in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
        // SAFETY: restoring SIG_DFL is always safe.
        let _ = unsafe { sigaction(sig, &action) };
    }
}
