mod image_downloader;
mod image_merger;

use crate::mount::{mount, FileAttr, MntArgs};
use anyhow::{Context, Result};
use image_downloader::ImageDownloader;
use image_merger::ImageMerger;
use nix::mount::MsFlags;
use rm_rf::remove;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};

const ROOTFS_NAME: &str = "rootfs";
const DEFAULT_WORKDIR: &str = "/tmp/orca/image";

pub struct Image {
    image_path: PathBuf,
    container_path: PathBuf,
    workdir_path: PathBuf,
    image_name: String,
    image_tag: String,
    container_name: String,
}

impl Image {
    pub fn new<S, T, U, V>(rootfs_prefix: S, image_name: T, image_tag: U, container_name: V) -> Self
    where
        S: AsRef<Path>,
        T: ToString,
        U: ToString,
        V: ToString,
    {
        let rootfs_prefix = rootfs_prefix.as_ref().to_path_buf();
        let image_name = image_name.to_string();
        let image_tag = image_tag.to_string();
        let container_name = container_name.to_string();
        let image_name_safe = if image_name.contains('/') {
            image_name.replace("/", "_")
        } else {
            image_name.clone()
        };
        let image_root = rootfs_prefix.join(&image_name_safe).join(&image_tag);
        let image_path = image_root.join(ROOTFS_NAME);
        let container_path = image_root.join(&container_name).join(ROOTFS_NAME);

        Self {
            image_path,
            container_path,
            workdir_path: AsRef::<Path>::as_ref(DEFAULT_WORKDIR).to_path_buf(),
            image_name,
            image_tag,
            container_name,
        }
    }

    pub fn workdir<T: AsRef<Path>>(&mut self, path: T) {
        self.workdir_path = path.as_ref().to_path_buf();
    }

    pub fn download(&self) -> Result<()> {
        let workdir = self.workdir_path.as_path();
        if workdir.exists() {
            remove(&self.workdir_path)
                .with_context(|| format!("Failed to remove: {}", workdir.display()))?;
        }
        create_dir_all(&self.workdir_path)
            .with_context(|| format!("Failed to create: {}", workdir.display()))?;

        let layers =
            ImageDownloader::new(&self.image_name, &self.image_tag, &self.image_path, workdir)
                .download_from_dockerhub()?;
        ImageMerger::new(&self.image_path)
            .add_layers(layers)
            .merge()?;
        remove(&self.workdir_path)
            .with_context(|| format!("Failed to remove: {}", workdir.display()))?;

        Ok(())
    }

    pub fn create_container_image(&self) -> Result<()> {
        let container = &self.container_path;
        let image = &self.image_path;
        create_dir_all(container)
            .with_context(|| format!("Failed to create dir: '{}'", container.display()))?;
        ImageMerger::new(container).add_layer(image).merge()
    }

    pub fn exists_image(&self) -> bool {
        self.image_path.exists()
    }

    pub fn exists_container(&self) -> bool {
        self.container_path.exists()
    }

    pub fn remove_image(&self) -> Result<()> {
        remove(&self.image_path)?;
        Ok(())
    }

    pub fn remove_container(&self) -> Result<()> {
        remove(&self.container_path)?;
        Ok(())
    }

    pub fn container_name(&self) -> String {
        self.container_name.clone()
    }

    pub fn image_root(&self) -> PathBuf {
        self.container_path.clone()
    }
}

pub trait ContainerImage {
    fn mount(&self) -> Result<()>;
    fn need_userns(&self) -> bool;
}

impl ContainerImage for Image {
    fn mount(&self) -> Result<()> {
        let continer_root = self.container_path.to_str().unwrap();
        let mnt_args = MntArgs::new(
            FileAttr::Dir,
            Some(continer_root),
            continer_root,
            None,
            MsFlags::MS_BIND,
            None,
        );
        mount(mnt_args).with_context(|| format!("Failed to bind mount '{}'", continer_root))?;
        Ok(())
    }

    fn need_userns(&self) -> bool {
        true
    }
}
