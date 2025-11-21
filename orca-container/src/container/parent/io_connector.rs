use anyhow::Result;
use nix::sys::epoll::*;
use nix::sys::eventfd::EventFd;
use nix::unistd::{read, write};
use std::os::fd::AsFd;
use std::thread;

pub struct IoConnector {
    handle: thread::JoinHandle<()>,
    shutdown: EventFd,
}

enum IOEvent {
    ParentStdin = 0,
    ChildStdout = 1,
    Shutdown = 2,
}

impl From<IOEvent> for u64 {
    fn from(event: IOEvent) -> Self {
        match event {
            IOEvent::ParentStdin => 0,
            IOEvent::ChildStdout => 1,
            IOEvent::Shutdown => 2,
        }
    }
}

impl TryFrom<u64> for IOEvent {
    type Error = ();

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(IOEvent::ParentStdin),
            1 => Ok(IOEvent::ChildStdout),
            2 => Ok(IOEvent::Shutdown),
            _ => Err(()),
        }
    }
}

impl IoConnector {
    pub fn new<F1, F2, F3, F4>(
        parent_stdout: F1,
        parent_stdin: F2,
        child_stdout: F3,
        child_stdin: F4,
    ) -> Self
    where
        F1: AsFd + Send + 'static,
        F2: AsFd + Send + 'static,
        F3: AsFd + Send + 'static,
        F4: AsFd + Send + 'static,
    {
        let epoll = Epoll::new(EpollCreateFlags::empty()).unwrap();
        let flags = EpollFlags::EPOLLIN;
        let shutdown = EventFd::new().unwrap();

        let event = EpollEvent::new(flags, IOEvent::ParentStdin.into());
        epoll.add(&parent_stdin, event).unwrap();
        let event = EpollEvent::new(flags, IOEvent::ChildStdout.into());
        epoll.add(&child_stdout, event).unwrap();
        let event = EpollEvent::new(flags, IOEvent::Shutdown.into());
        epoll.add(&shutdown, event).unwrap();

        let handle = thread::spawn(move || {
            let mut buffer = [0u8; 1024 * 1024];
            let mut events = [EpollEvent::empty(); 10];

            'outer: loop {
                let num_events = epoll.wait(&mut events, EpollTimeout::NONE).unwrap();
                for i in 0..num_events {
                    match events[i].data().try_into() {
                        // Data available on parent_stdin
                        Ok(IOEvent::ParentStdin) => match read(&parent_stdin, &mut buffer) {
                            Ok(bytes) if bytes > 0 => {
                                write(&child_stdin, &buffer[..bytes]).unwrap();
                            }
                            _ => {}
                        },

                        // Data available on child_stdout
                        Ok(IOEvent::ChildStdout) => match read(&child_stdout, &mut buffer) {
                            Ok(bytes) if bytes > 0 => {
                                write(&parent_stdout, &buffer[..bytes]).unwrap();
                            }
                            _ => {}
                        },

                        // Shutdown event
                        Ok(IOEvent::Shutdown) => break 'outer,

                        // This should never happen
                        Err(_) => panic!("Unknown IO event"),
                    }
                }
            }
        });

        Self { handle, shutdown }
    }

    pub fn stop(self) -> Result<()> {
        self.shutdown.write(1).unwrap();
        self.handle.join().unwrap();

        Ok(())
    }
}
