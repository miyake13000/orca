mod child;
mod parent;
mod terminal;

use crate::command::Command;
use crate::image::ContainerImage;
use crate::OrExit;
use crate::STACK_SIZE;
use anyhow::{Context, Result};
use nix::sched::{clone, CloneFlags};
use nix::sys::wait::wait;
use parent::io_connector::IoConnector;
use retry::{delay::Fixed, retry};
use std::ffi::CStr;
use std::io::stdin;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use terminal::Terminal;

pub struct Container<T> {
    image: T,
    io_connector: Option<IoConnector>,
    terminal: Terminal,
}

impl<T: ContainerImage> Container<T> {
    pub fn new<P>(
        image: T,
        command: String,
        cmd_args: Option<Vec<String>>,
        netns_flag: bool,
        work_dir: P,
    ) -> Result<Self>
    where
        T: ContainerImage,
        P: AsRef<Path>,
    {
        let stack: &mut [u8; STACK_SIZE] = &mut [0; STACK_SIZE];
        let cb = Box::new(|| child_main(&command, &cmd_args, &image, work_dir.as_ref()));
        let signals = Some(libc::SIGCHLD);

        let mut flags = CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWIPC
            | CloneFlags::CLONE_NEWPID;

        if image.need_userns() {
            flags.insert(CloneFlags::CLONE_NEWUSER);
        }
        if netns_flag {
            flags.insert(CloneFlags::CLONE_NEWNET);
        }

        let child_pid =
            clone(cb, stack, flags, signals).context("Failed to clone child process")?;

        if image.need_userns() {
            if Command::new("newuidmap", Option::<Vec<String>>::None).is_exist() {
                parent::Initilizer::map_id_with_subuid(child_pid)?;
            } else {
                parent::Initilizer::map_id(child_pid)?;
            }
        }

        parent::Initilizer::setns(child_pid, flags).context("Failed to enter namespace")?;

        let terminal = Terminal::new(stdin().as_raw_fd())?;

        Ok(Container {
            image,
            io_connector: None,
            terminal,
        })
    }

    pub fn connect_tty(&mut self) -> Result<()> {
        self.io_connector = Some(parent::Initilizer::connect_tty()?);
        let win_size = self
            .terminal
            .get_win_size()
            .context("Failed to get current window size")?;
        let pty_slave = retry(Fixed::from_millis(10).take(100), || {
            nix::fcntl::open(
                "/dev/pts/0",
                nix::fcntl::OFlag::O_RDWR,
                nix::sys::stat::Mode::empty(),
            )
        })
        .context("Failed to open /dev/pts/0")?;

        Terminal::new(pty_slave)
            .context("Failed to open pty_slave")?
            .set_win_size(win_size)
            .context("Failed to set window size")?;
        self.terminal.make_raw_mode()?;

        Ok(())
    }

    pub fn wait(self) -> Result<T> {
        wait().context("Failed to wait child process")?;
        if let Some(io_connector) = self.io_connector {
            io_connector.stop()?;
        }
        Ok(self.image)
    }
}

#[allow(clippy::needless_return)]
fn child_main<T, U, I>(command: T, cmd_args: &Option<Vec<U>>, image: &I, work_dir: &Path) -> isize
where
    T: AsRef<str>,
    U: AsRef<str> + Clone,
    I: ContainerImage,
{
    use child::Initializer;
    let error_message = "Failed to initialize container";

    if image.need_userns() {
        Initializer::wait_for_mapping_id()
            .context(error_message)
            .or_exit();
    }
    Initializer::store_resolv_conf(work_dir)
        .context(error_message)
        .or_exit();
    image.mount().context(error_message).or_exit();
    Initializer::pivot_root(image.root_path())
        .context(error_message)
        .or_exit();
    Initializer::mount_mandatory_files()
        .context(error_message)
        .or_exit();
    Initializer::copy_resolv_conf(work_dir)
        .context(error_message)
        .or_exit();
    Initializer::create_ptmx_link()
        .context(error_message)
        .or_exit();
    if image.need_userns() {
        Initializer::set_hostname(image.name())
            .context(error_message)
            .or_exit();
    }
    Initializer::connect_tty().context(error_message).or_exit();
    Initializer::unmount_old_root()
        .context(error_message)
        .or_exit();

    let envp: Vec<&CStr> = vec![
        CStr::from_bytes_with_nul(b"SHELL=/bin/sh\0").unwrap(),
        CStr::from_bytes_with_nul(b"HOME=/root\0").unwrap(),
        CStr::from_bytes_with_nul(b"TERM=xterm\0").unwrap(),
        CStr::from_bytes_with_nul(b"PATH=/bin:/usr/bin:/sbin:/usr/sbin\0").unwrap(),
    ];

    Initializer::exec(command, cmd_args.clone(), &envp)
        .context("Failed to initialize container")
        .or_exit();

    return 0; // This unreachable code is neccessary for CloneCb
}
