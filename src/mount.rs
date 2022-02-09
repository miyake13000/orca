use anyhow::{bail, Context, Result};
use nix::mount::{MntFlags, MsFlags};
use rm_rf::remove;
use std::fs::{create_dir_all, File};
use std::path::Path;

#[derive(Eq, PartialEq, Ord, PartialOrd)]
pub enum FileType {
    File,
    Dir,
}

impl FileType {
    pub fn is_file(&self) -> bool {
        *self == FileType::File
    }

    pub fn is_dir(&self) -> bool {
        *self == FileType::Dir
    }
}

pub type MountFlags = MsFlags;

pub struct Mount<P1, P2> {
    src: Option<P1>,
    dest: P1,
    fs_type: Option<P2>,
    flags: MountFlags,
    data: Option<P2>,
    file_type: FileType,
}

impl<P1, P2> Mount<P1, P2>
where
    P1: AsRef<Path>,
    P2: AsRef<str>,
{
    pub fn new(dest: P1, file_type: FileType) -> Self {
        Self {
            src: None,
            dest,
            fs_type: None,
            flags: MountFlags::empty(),
            data: None,
            file_type,
        }
    }

    pub fn src(mut self, src: P1) -> Self {
        self.src = Some(src);
        self
    }

    pub fn fs_type(mut self, fs_type: P2) -> Self {
        self.fs_type = Some(fs_type);
        self
    }

    pub fn data(mut self, data: P2) -> Self {
        self.data = Some(data);
        self
    }

    pub fn flags(mut self, flags: MountFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn add_flags(mut self, flag: MountFlags) -> Self {
        self.flags = self.flags.union(flag);
        self
    }

    pub fn mount(self) -> Result<()> {
        let dest_path = self.dest.as_ref();
        let metadata = dest_path.metadata();

        if let Ok(dest_file_type) = metadata {
            if self.file_type.is_dir() && !dest_file_type.is_dir() {
                bail!("Cannot mount directory on file: {}", dest_path.display());
            } else if self.file_type.is_file() && dest_file_type.is_dir() {
                bail!("Cannot mount file on directory: {}", dest_path.display());
            }
        } else if metadata.is_err() {
            if self.file_type.is_file() {
                File::create(dest_path)
                    .with_context(|| format!("Failed to create '{}'", dest_path.display()))?;
            } else {
                create_dir_all(dest_path)
                    .with_context(|| format!("Failed to create '{}'", dest_path.display()))?;
            }
        }

        nix::mount::mount(
            self.src.as_ref().map(|o| o.as_ref()),
            self.dest.as_ref(),
            self.fs_type.as_ref().map(|o| o.as_ref()),
            self.flags,
            self.data.as_ref().map(|o| o.as_ref()),
        )?;

        Ok(())
    }
}

pub type UnMountFlags = MntFlags;

pub struct UnMount<T> {
    dest: T,
    flags: UnMountFlags,
    remove_mount_point: bool,
}

impl<T: AsRef<Path>> UnMount<T> {
    pub fn new(dest: T) -> Self {
        Self {
            dest,
            flags: UnMountFlags::empty(),
            remove_mount_point: false,
        }
    }

    pub fn flags(mut self, flags: UnMountFlags) -> Self {
        self.flags = flags;
        self
    }

    pub fn add_flag(mut self, flag: UnMountFlags) -> Self {
        self.flags = self.flags | flag;
        self
    }

    pub fn remove_mount_point(mut self, flag: bool) -> Self {
        self.remove_mount_point = flag;
        self
    }

    pub fn unmount(self) -> Result<()> {
        let dest = self.dest.as_ref();
        nix::mount::umount2(dest, self.flags)?;

        if self.remove_mount_point {
            if dest.is_file() || dest.is_dir() {
                remove(dest)?;
            } else {
                bail!("Cannot remove: '{}'", dest.display());
            }
        }

        Ok(())
    }
}
