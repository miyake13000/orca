use anyhow::Result;
use rm_rf::remove;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};

const ROOTFS_NAME: &str = "rootfs";
const UPPERDIR_NAME: &str = "upper";
const WORKDIR_NAME: &str = "work";
const IMAGE_NAME: &str = "host_image";

pub struct HostImage {
    pub ovr_upperdir: PathBuf,
    pub ovr_workdir: PathBuf,
    container_path: PathBuf,
    container_name: String,
}

impl HostImage {
    pub fn new<S, T>(rootfs_prefix: S, container_name: T) -> Self
    where
        S: AsRef<Path>,
        T: ToString,
    {
        let rootfs_prefix = rootfs_prefix.as_ref().to_path_buf();
        let container_name = container_name.to_string();
        let container_root = rootfs_prefix.join(IMAGE_NAME).join(&container_name);
        let container_path = container_root.join(ROOTFS_NAME);
        let ovr_upperdir = container_root.join(UPPERDIR_NAME);
        let ovr_workdir = container_root.join(WORKDIR_NAME);

        Self {
            ovr_upperdir,
            ovr_workdir,
            container_path,
            container_name,
        }
    }

    pub fn create(&self) -> Result<()> {
        create_dir_all(&self.ovr_upperdir)?;
        create_dir_all(&self.ovr_workdir)?;
        create_dir_all(&self.container_path)?;
        Ok(())
    }

    pub fn container_path(&self) -> PathBuf {
        self.container_path.clone()
    }

    pub fn exists_container(&self) -> bool {
        self.ovr_upperdir.exists()
    }

    pub fn remove_container(&self) -> Result<()> {
        remove(&self.ovr_upperdir)?;
        Ok(())
    }

    pub fn container_name(&self) -> String {
        self.container_name.clone()
    }
}
