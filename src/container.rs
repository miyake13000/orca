mod child;
mod parent;
mod terminal;

use crate::image::ContainerImage;
use crate::OrExit;
use crate::STACK_SIZE;
use anyhow::bail;
use anyhow::{Context, Result};
use nix::libc::SIGCHLD;
use nix::sched::{clone, CloneFlags};
use nix::sys::wait::wait;
use os_pipe::pipe;
use os_pipe::PipeReader;
use os_pipe::PipeWriter;
use parent::io_connector::IoConnector;
use std::io::stdin;
use std::io::{Read, Write};
use std::path::Path;
use terminal::Terminal;

const SIGNAL_MOUNTED_DEVPTS: &[u8] = b"1";
const SIGNAL_OPEN_PTMX: &[u8] = b"2";

pub struct Container<T> {
    image: T,
    io_connector: IoConnector,
    terminal: Terminal,
}

impl<T: ContainerImage> Container<T> {
    pub fn new<P>(image: T, command: Vec<String>, work_dir: P) -> Result<Self>
    where
        T: ContainerImage,
        P: AsRef<Path>,
    {
        let stack: &mut [u8; STACK_SIZE] = &mut [0; STACK_SIZE];
        let (mut child_reader, mut parent_writer) = pipe().unwrap();
        let (mut parent_reader, mut child_writer) = pipe().unwrap();
        let cb = Box::new(|| {
            child_main(
                &command,
                &image,
                work_dir.as_ref(),
                &mut child_reader,
                &mut child_writer,
            )
        });
        let signals = Some(SIGCHLD);
        let flags = CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWIPC;
        let child_pid =
            unsafe { clone(cb, stack, flags, signals).context("Failed to clone child process")? };

        parent::Initilizer::setns(child_pid, flags).context("Failed to enter namespace")?;

        let mut signal_buf: [u8; 1] = [b'0'; 1];
        parent_reader.read_exact(&mut signal_buf).unwrap();
        if signal_buf != SIGNAL_MOUNTED_DEVPTS {
            bail!("Unexpected child signal has received: {}", signal_buf[0]);
        }
        let io_connector = parent::Initilizer::connect_tty()?;
        parent_writer.write_all(SIGNAL_OPEN_PTMX).unwrap();
        let mut terminal = Terminal::new(stdin())?;
        terminal.make_raw_mode()?;
        let mut slave_terminal = get_pty_slave()?;
        sync_tty_size(&terminal, &mut slave_terminal)?;

        Ok(Container {
            image,
            io_connector,
            terminal,
        })
    }

    pub fn wait(self) -> Result<T> {
        wait().context("Failed to wait child process")?;
        self.io_connector.stop()?;
        std::mem::drop(self.terminal);
        Ok(self.image)
    }
}

fn get_pty_slave() -> Result<Terminal> {
    let pty_slave = nix::fcntl::open(
        "/dev/pts/0",
        nix::fcntl::OFlag::O_RDWR,
        nix::sys::stat::Mode::empty(),
    )
    .context("Failed to open /dev/pts/0")?;

    Terminal::new(pty_slave).context("Failed to open pty_slave")
}

fn sync_tty_size(src: &Terminal, tar: &mut Terminal) -> Result<()> {
    let mut win_size = src
        .get_win_size()
        .context("Failed to get current window size")?;
    tar.set_win_size(&mut win_size)
        .context("Failed to set window size")?;

    Ok(())
}

#[allow(clippy::needless_return)]
fn child_main<T, I>(
    command: &[T],
    image: &I,
    work_dir: &Path,
    reader: &mut PipeReader,
    writer: &mut PipeWriter,
) -> isize
where
    T: AsRef<str>,
    I: ContainerImage,
{
    use child::Initializer;
    let error_message = "Failed to initialize container";
    let mut signal_buf: [u8; 1] = [b'0'; 1];

    Initializer::store_resolv_conf(work_dir)
        .context(error_message)
        .or_exit();
    image.mount().context(error_message).or_exit();
    Initializer::pivot_root(image.rootfs_path())
        .context(error_message)
        .or_exit();
    Initializer::mount_mandatory_files()
        .context(error_message)
        .or_exit();
    writer.write_all(SIGNAL_MOUNTED_DEVPTS).unwrap();
    Initializer::copy_resolv_conf(work_dir)
        .context(error_message)
        .or_exit();
    Initializer::create_ptmx_link()
        .context(error_message)
        .or_exit();
    reader.read_exact(&mut signal_buf).unwrap();
    if signal_buf != SIGNAL_OPEN_PTMX {
        Result::<String>::Err(anyhow::anyhow!(
            "Unexpected parent signal has received: {}",
            signal_buf[0]
        ))
        .context(error_message)
        .or_exit();
    }
    Initializer::connect_tty().context(error_message).or_exit();
    Initializer::unmount_old_root()
        .context(error_message)
        .or_exit();
    Initializer::exec(command)
        .context("Failed to initialize container")
        .or_exit();

    return 0; // This return is unreadchable but neccessary for CloneCb
}
