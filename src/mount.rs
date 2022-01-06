use anyhow::{anyhow, Context, Result};
use nix::mount::{MntFlags, MsFlags};
use std::fs::{create_dir_all, metadata, File};

pub fn mount(args: MntArgs) -> Result<()> {
    let metadata = metadata(args.dest);

    if metadata.is_err() {
        if args.is_file() {
            File::create(args.dest).with_context(|| format!("Failed to create '{}'", args.dest))?;
        } else if args.is_dir() {
            create_dir_all(args.dest)
                .with_context(|| format!("Failed to create '{}'", args.dest))?;
        } else {
            return Err(anyhow!("cannot mount special file"));
        }
    } else if metadata.is_ok() {
        let file_attr = metadata.unwrap();
        if args.is_file() {
            if !file_attr.is_file() {
                return Err(anyhow!("src is file but dest is not file"));
            }
        } else if args.is_dir() {
            if !file_attr.is_dir() {
                return Err(anyhow!("src is dir but dest is not dir"));
            }
        } else {
            return Err(anyhow!("dest is neither file nor dir"));
        }
    }

    nix::mount::mount(args.src, args.dest, args.fstype, args.ms_flags, args.data)?;

    Ok(())
}

pub fn umount(args: UMntArgs) -> Result<()> {
    nix::mount::umount2(args.dest, args.mnt_flags)?;

    Ok(())
}

#[derive(Eq, PartialEq, Ord, PartialOrd)]
pub enum FileAttr {
    File,
    Dir,
    Ohter,
}

pub struct MntArgs<'a> {
    file_attr: FileAttr,
    src: Option<&'a str>,
    dest: &'a str,
    fstype: Option<&'a str>,
    ms_flags: MsFlags,
    data: Option<&'a str>,
}

impl<'a> MntArgs<'a> {
    pub fn new(
        file_attr: FileAttr,
        src: Option<&'a str>,
        dest: &'a str,
        fstype: Option<&'a str>,
        ms_flags: MsFlags,
        data: Option<&'a str>,
    ) -> Self {
        Self {
            file_attr,
            src,
            dest,
            fstype,
            ms_flags,
            data,
        }
    }
    pub fn is_file(&self) -> bool {
        if self.file_attr == FileAttr::File {
            true
        } else {
            false
        }
    }

    pub fn is_dir(&self) -> bool {
        if self.file_attr == FileAttr::Dir {
            true
        } else {
            false
        }
    }
}

pub struct UMntArgs<'a> {
    file_attr: FileAttr,
    dest: &'a str,
    mnt_flags: MntFlags,
}

impl<'a> UMntArgs<'a> {
    pub fn new(file_attr: FileAttr, dest: &'a str, mnt_flags: MntFlags) -> Self {
        Self {
            file_attr,
            dest,
            mnt_flags,
        }
    }

    pub fn is_file(&self) -> bool {
        if self.file_attr == FileAttr::File {
            true
        } else {
            false
        }
    }

    pub fn is_dir(&self) -> bool {
        if self.file_attr == FileAttr::Dir {
            true
        } else {
            false
        }
    }
}

pub const PROC: MntArgs = MntArgs {
    file_attr: FileAttr::Dir,
    src: Some("proc"),
    dest: "/proc",
    fstype: Some("proc"),
    ms_flags: MsFlags::from_bits_truncate(
        MsFlags::MS_NODEV.bits() | MsFlags::MS_NOSUID.bits() | MsFlags::MS_NOEXEC.bits(),
    ),
    data: None,
};

pub const DEV: MntArgs = MntArgs {
    file_attr: FileAttr::Dir,
    src: Some("tmpfs"),
    dest: "/dev",
    fstype: Some("tmpfs"),
    ms_flags: MsFlags::MS_NOSUID,
    data: Some("mode=755"),
};

pub const DEVPTS: MntArgs = MntArgs {
    file_attr: FileAttr::Dir,
    src: Some("devpts"),
    dest: "/dev/pts",
    fstype: Some("devpts"),
    ms_flags: MsFlags::from_bits_truncate(MsFlags::MS_NOSUID.bits() | MsFlags::MS_NOEXEC.bits()),
    data: Some("mode=620,ptmxmode=666"),
};

pub const SYSFS: MntArgs = MntArgs {
    file_attr: FileAttr::Dir,
    src: Some("sysfs"),
    dest: "/sys",
    fstype: None,
    ms_flags: MsFlags::from_bits_truncate(
        MsFlags::MS_RDONLY.bits()
            | MsFlags::MS_NOSUID.bits()
            | MsFlags::MS_NODEV.bits()
            | MsFlags::MS_NOEXEC.bits(),
    ),
    data: None,
};

pub const MQUEUE: MntArgs = MntArgs {
    file_attr: FileAttr::Dir,
    src: Some("mqueue"),
    dest: "/dev/mqueue",
    fstype: Some("mqueue"),
    ms_flags: MsFlags::from_bits_truncate(
        MsFlags::MS_NODEV.bits() | MsFlags::MS_NOSUID.bits() | MsFlags::MS_NOEXEC.bits(),
    ),
    data: None,
};

pub const SHM: MntArgs = MntArgs {
    file_attr: FileAttr::Dir,
    src: Some("shm"),
    dest: "/dev/shm",
    fstype: Some("tmpfs"),
    ms_flags: MsFlags::from_bits_truncate(
        MsFlags::MS_NODEV.bits() | MsFlags::MS_NOSUID.bits() | MsFlags::MS_NOEXEC.bits(),
    ),
    data: None,
};

pub const DEVNULL: MntArgs = MntArgs {
    file_attr: FileAttr::File,
    src: Some("/oldroot/dev/null"),
    dest: "/dev/null",
    fstype: None,
    ms_flags: MsFlags::MS_BIND,
    data: None,
};

pub const DEVRANDOM: MntArgs = MntArgs {
    file_attr: FileAttr::File,
    src: Some("/oldroot/dev/random"),
    dest: "/dev/random",
    fstype: None,
    ms_flags: MsFlags::MS_BIND,
    data: None,
};
pub const DEVFULL: MntArgs = MntArgs {
    file_attr: FileAttr::File,
    src: Some("/oldroot/dev/full"),
    dest: "/dev/full",
    fstype: None,
    ms_flags: MsFlags::MS_BIND,
    data: None,
};

pub const DEVTTY: MntArgs = MntArgs {
    file_attr: FileAttr::File,
    src: Some("/oldroot/dev/tty"),
    dest: "/dev/tty",
    fstype: None,
    ms_flags: MsFlags::MS_BIND,
    data: None,
};

pub const DEVZERO: MntArgs = MntArgs {
    file_attr: FileAttr::File,
    src: Some("/oldroot/dev/zero"),
    dest: "/dev/zero",
    fstype: None,
    ms_flags: MsFlags::MS_BIND,
    data: None,
};

pub const DEVURANDOM: MntArgs = MntArgs {
    file_attr: FileAttr::File,
    src: Some("/oldroot/dev/urandom"),
    dest: "/dev/urandom",
    fstype: None,
    ms_flags: MsFlags::MS_BIND,
    data: None,
};

pub const DEVCONSOLE: MntArgs = MntArgs {
    file_attr: FileAttr::File,
    src: Some("/dev/pts/0"),
    dest: "/dev/console",
    fstype: Some("proc"),
    ms_flags: MsFlags::MS_BIND,
    data: None,
};

pub const OLDROOT: UMntArgs = UMntArgs {
    file_attr: FileAttr::Dir,
    dest: "/oldroot",
    mnt_flags: MntFlags::MNT_DETACH,
};
