use crate::mount::{FileType, Mount, MountFlags};
use anyhow::{Context, Result};
use std::{
    fs::create_dir_all,
    path::{Path, PathBuf},
};

const OVERLAYFS_FSTYPE: &str = "overlay";

pub struct HostImage {
    mount_config: OverlayConfig,
    fake_mount_config: OverlayConfig,
}

struct OverlayConfig {
    mp: PathBuf,
    upperdir: PathBuf,
    lowerdir: Vec<PathBuf>,
    workdir: PathBuf,
}

pub trait ContainerImage {
    fn mount(&self) -> Result<()>;
    fn rootfs_path(&self) -> &Path;
}

impl HostImage {
    pub fn new<S1, S2, S3, S4, S5>(
        mount_point: S1,
        upperdir: S2,
        additional_lowerdirs: Vec<S3>,
        workdir: S4,
        tmpdir: S5,
    ) -> Self
    where
        S1: Into<PathBuf>,
        S2: Into<PathBuf>,
        S3: Into<PathBuf>,
        S4: Into<PathBuf>,
        S5: Into<PathBuf>,
    {
        let tmpdir = tmpdir.into();
        let fake_mount_config = OverlayConfig {
            mp: tmpdir.join("fake_rootfs"),
            upperdir: tmpdir.join("fake_upper"),
            lowerdir: vec![PathBuf::from("/")],
            workdir: tmpdir.join("fake_work"),
        };
        create_all_dirs(&fake_mount_config);

        let mut lowerdir: Vec<PathBuf> =
            additional_lowerdirs.into_iter().map(|p| p.into()).collect();
        lowerdir.push(fake_mount_config.mp.clone());

        let mount_config = OverlayConfig {
            mp: mount_point.into(),
            upperdir: upperdir.into(),
            lowerdir,
            workdir: workdir.into(),
        };

        Self {
            mount_config,
            fake_mount_config,
        }
    }
}

fn create_all_dirs(dirs: &OverlayConfig) {
    if !dirs.mp.exists() {
        create_dir_all(&dirs.mp).unwrap();
    }
    if !dirs.upperdir.exists() {
        create_dir_all(&dirs.upperdir).unwrap();
    }
    if !dirs.workdir.exists() {
        create_dir_all(&dirs.workdir).unwrap();
    }
}

impl ContainerImage for HostImage {
    fn mount(&self) -> Result<()> {
        Mount::new("/", FileType::Dir)
            .add_flags(MountFlags::MS_PRIVATE)
            .add_flags(MountFlags::MS_REC)
            .mount()
            .context("Failed to make '/' private")?;

        Mount::new(&self.fake_mount_config.mp, FileType::Dir)
            .fs_type(OVERLAYFS_FSTYPE)
            .data(self.fake_mount_config.to_option_string().as_str())
            .mount()?;

        Mount::new(&self.mount_config.mp, FileType::Dir)
            .fs_type(OVERLAYFS_FSTYPE)
            .data(self.mount_config.to_option_string().as_str())
            .mount()
    }

    fn rootfs_path(&self) -> &Path {
        self.mount_config.mp.as_path()
    }
}

impl OverlayConfig {
    fn to_option_string(&self) -> String {
        let upperdir = format!("{}", self.upperdir.display());
        let workdir = format!("{}", self.workdir.display());
        let mut lowerdir = String::new();
        self.lowerdir
            .iter()
            .for_each(|p| lowerdir.push_str(format!("{}:", p.display()).as_str()));
        let lowerdir_striped = if !lowerdir.is_empty() {
            lowerdir.strip_suffix(':').unwrap()
        } else {
            lowerdir.as_str()
        };
        format!("lowerdir={lowerdir_striped},upperdir={upperdir},workdir={workdir}")
    }
}
