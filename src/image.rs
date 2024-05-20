// mod guest_image;
mod host_image;

// pub use guest_image::GuestImage;
pub use host_image::HostImage;

use anyhow::Result;
use std::path::Path;

pub trait ContainerImage {
    fn mount(&self) -> Result<()>;
    fn rootfs_path(&self) -> &Path;
}
