use anyhow::{bail, Context, Result};
use nix::mount::{MntFlags, MsFlags};
use std::fs::{create_dir_all, remove_dir_all, File};
use std::path::{Path, PathBuf};

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

pub struct Mount {
    src: Option<PathBuf>,
    dest: PathBuf,
    fs_type: Option<String>,
    flags: MountFlags,
    data: Option<String>,
    file_type: FileType,
}

impl Mount {
    pub fn new<P: Into<PathBuf>>(dest: P, file_type: FileType) -> Self {
        Self {
            src: None,
            dest: dest.into(),
            fs_type: None,
            flags: MountFlags::empty(),
            data: None,
            file_type,
        }
    }

    pub fn src<P: Into<PathBuf>>(mut self, src: P) -> Self {
        self.src = Some(src.into());
        self
    }

    pub fn fs_type<S: ToString>(mut self, fs_type: S) -> Self {
        self.fs_type = Some(fs_type.to_string());
        self
    }

    pub fn data<S: ToString>(mut self, data: S) -> Self {
        self.data = Some(data.to_string());
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
        let dest_path = self.dest.as_path();
        match dest_path.metadata() {
            Ok(dest) => {
                if self.file_type.is_dir() && !dest.is_dir() {
                    bail!("Cannot mount directory on file: {}", dest_path.display());
                } else if self.file_type.is_file() && dest.is_dir() {
                    bail!("Cannot mount file on directory: {}", dest_path.display());
                }
            }
            Err(_) => {
                if self.file_type.is_file() {
                    File::create(dest_path)
                        .with_context(|| format!("Failed to create '{}'", dest_path.display()))?;
                } else {
                    create_dir_all(dest_path)
                        .with_context(|| format!("Failed to create '{}'", dest_path.display()))?;
                }
            }
        }

        nix::mount::mount(
            self.src.as_deref(),
            self.dest.as_path(),
            self.fs_type.as_deref(),
            self.flags,
            self.data.as_deref(),
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
        self.flags |= flag;
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
                remove_dir_all(dest)?;
            } else {
                bail!("Cannot remove: '{}'", dest.display());
            }
        }

        Ok(())
    }
}
