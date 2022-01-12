use anyhow::{Context, Result};
use flate2::bufread::MultiGzDecoder;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{copy, BufReader};
use std::path::Path;
use std::process::Command;
use tar::Archive;

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Res1 {
    token: String,
    access_token: String,
    expires_in: u32,
    issued_at: String,
}

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Config {
    mediaType: String,
    size: usize,
    digest: String,
}

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Res2 {
    schemaVersion: usize,
    mediaType: String,
    config: Config,
    layers: Vec<Config>,
}

pub struct Image {
    pub tarball_path: String,
    pub rootfs_path: String,
    pub image_name: String,
    pub image_tag: String,
}

impl Image {
    pub fn new(
        rootfs_prefix: String,
        image_name: String,
        image_tag: String,
        container_name: String,
    ) -> Self {
        let image_name_safe = if image_name.contains('/') {
            image_name.replace("/", "_")
        } else {
            image_name
        };

        let image_root_path = format!("{}/{}/{}", rootfs_prefix, image_name_safe, image_tag);
        let tarball_path = format!("{}/image.tar.gz", image_root_path);
        let rootfs_path = format!("{}/{}/rootfs", image_root_path, container_name);
        fs::create_dir_all(image_root_path).unwrap();

        Image {
            tarball_path,
            rootfs_path,
            image_name: image_name_safe,
            image_tag,
        }
    }

    pub fn image_exists(&self) -> bool {
        Path::new(&self.tarball_path).exists()
    }

    pub fn container_exists(&self) -> bool {
        Path::new(&self.rootfs_path).exists()
    }

    pub fn download(&self) -> Result<()> {
        let token = Self::get_token(&self.image_name)?;
        let layer_id = Self::get_layer_id(&self.image_name, &self.image_tag, &token)?;
        Self::download_layer_tarball(&self.image_name, &token, &layer_id, &self.tarball_path)?;
        Ok(())
    }

    pub fn extract(&self) -> Result<()> {
        fs::create_dir_all(&self.rootfs_path)
            .with_context(|| format!("Failed to create '{}'", self.rootfs_path))?;
        let tar_gz = File::open(&self.tarball_path)
            .with_context(|| format!("Failed to open '{}", self.tarball_path))?;
        let reader = BufReader::new(tar_gz);
        let tar = MultiGzDecoder::new(reader);
        let mut archive = Archive::new(tar);
        archive
            .unpack(&self.rootfs_path)
            .context("Failed to unpack tarball")?;

        Ok(())
    }

    pub fn remove(&self) -> Result<()> {
        let _ = Command::new("rm")
            .arg("-rf")
            .arg(&self.rootfs_path)
            .status()?;
        Ok(())
    }

    fn get_token(image_name: &str) -> Result<String> {
        let url = format!("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/{}:pull", image_name);
        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(&url)
            .send()
            .context("Failed to get bearer token")?
            .text()
            .context("Failed to convert response to string")?;
        let res_json: Res1 =
            serde_json::from_str(&resp).context("Failed to convert response to json")?;
        Ok(res_json.token)
    }

    fn get_layer_id(image_name: &str, image_tag: &str, token: &str) -> Result<String> {
        let url = format!(
            "https://registry-1.docker.io/v2/library/{}/manifests/{}",
            image_name, image_tag
        );
        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(&url)
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .bearer_auth(token)
            .send()
            .context("Failed to get image id")?
            .text()
            .context("Failed to convert resopnse to string")?;
        let res_json: Res2 =
            serde_json::from_str(&resp).context("Failed to convert response to json")?;
        Ok(res_json.layers[0].digest.to_string())
    }

    #[allow(dead_code)]
    fn get_var(
        token: &str,
        image_name: &str,
        image_id: &str,
    ) -> std::result::Result<(String, String), ()> {
        let url = format!(
            "https://registry-1.docker.io/v2/library/{}/blobs/{}",
            image_name, image_id
        );

        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(&url)
            .bearer_auth(token)
            .send()
            .unwrap()
            .text()
            .unwrap();
        let _res_json: Res2 = serde_json::from_str(&resp).unwrap();
        Ok((String::from(""), String::from("")))
    }

    fn download_layer_tarball(
        image_name: &str,
        token: &str,
        layer_id: &str,
        file_path: &str,
    ) -> Result<()> {
        let url = format!(
            "https://registry-1.docker.io/v2/library/{}/blobs/{}",
            image_name, layer_id
        );
        let client = reqwest::blocking::Client::new();
        let mut resp = client
            .get(&url)
            .bearer_auth(token)
            .send()
            .context("Failed to donwload container image")?;

        let mut file = File::create(file_path)
            .with_context(|| format!("Failed to create file: '{}'", file_path))?;
        copy(&mut resp, &mut file)
            .with_context(|| format!("Failed to write container image data to '{}'", file_path))?;
        Ok(())
    }
}
