// This file has below struct and impl
// ・Image : operate container image

extern crate reqwest;
extern crate serde;
extern crate serde_json;

use serde::{Deserialize, Serialize};
use flate2::read::GzDecoder;
use tar::Archive;
use std::path::{Path, PathBuf};
use std::io::{copy, Result};
use std::fs::{File, create_dir_all};

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
    pub dest_name: String,
    pub dest_tag:  String,
    pub path_image: PathBuf,
    pub path_rootfs: PathBuf,
}

impl Image {
    pub fn new(name: String, tag: String, path: String) -> Image {
        let mut path_image = PathBuf::from(&path);
        let mut path_rootfs = PathBuf::from(&path);
        path_image.push("image.tar.gz");
        path_rootfs.push("rootfs");

        Image{
            dest_name: name,
            dest_tag: tag,
            path_image: path_image,
            path_rootfs: path_rootfs
        }
    }

    pub fn exist_image(&self) -> bool {
        Path::new(self.path_image.as_path()).exists()
    }

    pub fn exist_rootfs(&self) -> bool {
        Path::new(self.path_rootfs.as_path()).exists()
    }

    pub fn create_dir(&self) -> Result<()> {
        create_dir_all(self.path_rootfs.as_path())
    }

    pub fn get(&self) -> Result<()> {
        let url = format!("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/{}:pull", self.dest_name);
        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url)
            .send()
            .unwrap()
            .text()
            .unwrap();
        let res_json: Res1 = serde_json::from_str(&resp).unwrap();
        let token = &res_json.token;

        let url = format!("https://registry-1.docker.io/v2/library/{}/manifests/{}", self.dest_name, self.dest_tag);
        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.docker.distribution.manifest.v2+json")
            .bearer_auth(token)
            .send()
            .unwrap()
            .text()
            .unwrap();
        let res_json: Res2 = serde_json::from_str(&resp).unwrap();
        let layer_id = &res_json.layers[0].digest;

        let url = format!("https://registry-1.docker.io/v2/library/{}/blobs/{}", self.dest_name, layer_id);
        let client = reqwest::blocking::Client::new();
        let mut resp = client.get(&url)
            .bearer_auth(token)
            .send()
            .unwrap();

        let mut file = File::create(self.path_image.as_path()).expect("file create");
        copy(&mut resp, &mut file).unwrap();
        Ok(())
    }

    pub fn extract(&self) -> Result<()> {
        let tar_gz = File::open(self.path_image.as_path()).expect("file open");
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        archive.unpack(self.path_rootfs.as_path())
    }
}
