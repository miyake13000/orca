mod downloader;
mod merger;

use anyhow::{Context, Result};
use downloader::ImageDownloader;
use merger::ImageMerger;
use rm_rf::remove;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};

const ROOTFS_NAME: &str = "rootfs";
const DEFAULT_WORKDIR: &str = "/tmp/image_downloader/";

pub struct GuestImage {
    image_path: PathBuf,
    container_path: PathBuf,
    workdir_path: PathBuf,
    image_name: String,
    image_tag: String,
    container_name: String,
    display_progress: bool,
}

impl GuestImage {
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
            display_progress: false,
        }
    }

    pub fn workdir<T: AsRef<Path>>(mut self, path: T) -> Self {
        self.workdir_path = path.as_ref().to_path_buf();
        self
    }

    pub fn display_progress(mut self, b: bool) -> Self {
        self.display_progress = b;
        self
    }

    pub fn download(&self) -> Result<()> {
        let workdir = self.workdir_path.as_path();
        if workdir.exists() {
            remove(&self.workdir_path)
                .with_context(|| format!("Failed to remove: {}", workdir.display()))?;
        }
        create_dir_all(&self.workdir_path)
            .with_context(|| format!("Failed to create: {}", workdir.display()))?;

        let mut image_downloader =
            ImageDownloader::new(&self.image_name, &self.image_tag, &self.image_path, workdir);
        if self.display_progress {
            image_downloader.pre_download_display(display_pre_download);
            image_downloader.post_download_display(display_post_download);
        }
        let layers = image_downloader.download_from_dockerhub()?;
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

    pub fn container_path(&self) -> PathBuf {
        self.container_path.clone()
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

fn display_pre_download(name: &str, tag: &str) {
    println!("Download container image: {}:{} ", name, tag);
}

fn display_post_download(num_of_layer: usize, downloaded_layer: usize) {
    if downloaded_layer == 0 {
        println!();
    }
    print!("\r\x1b[1A\x1b[K");
    println!("{}/{} layer has downloaded", downloaded_layer, num_of_layer);
}
