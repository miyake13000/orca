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
    tarball_path: String,
    pub rootfs_path: String,
    pub image_name: String,
    pub image_tag: String,
}

impl Image {
    pub fn new(root_path: String, image_name: String, image_tag: String) -> Self {
        let tarball_path = format!("{}/image.tar.gz", &root_path);
        let rootfs_path = format!("{}/rootfs", &root_path);
        fs::create_dir_all(&root_path).unwrap();
        Image {
            tarball_path,
            rootfs_path,
            image_name,
            image_tag,
        }
    }

    pub fn exist(&self) -> bool {
        Path::new(&self.rootfs_path).exists()
    }

    pub fn download(&self) -> std::result::Result<(), ()> {
        let token = Self::get_token(&self.image_name).unwrap();
        let layer_id = Self::get_layer_id(&self.image_name, &self.image_tag, &token).unwrap();
        Self::download_layer_tarball(&self.image_name, &token, &layer_id, &self.tarball_path)
            .unwrap();
        Ok(())
    }

    pub fn extract(&self) -> std::result::Result<(), ()> {
        fs::create_dir_all(&self.rootfs_path).unwrap();
        let tar_gz = File::open(&self.tarball_path).unwrap();
        let reader = BufReader::new(tar_gz);
        let tar = MultiGzDecoder::new(reader);
        let mut archive = Archive::new(tar);
        archive.unpack(&self.rootfs_path).unwrap();

        Ok(())
    }

    pub fn remove(&self) -> std::result::Result<(), ()> {
        let _ = Command::new("rm")
            .arg("-rf")
            .arg(&self.rootfs_path)
            .output()
            .unwrap();
        Ok(())
    }

    fn get_token(image_name: &str) -> std::result::Result<String, ()> {
        let url = format!("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/{}:pull", image_name);
        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url).send().unwrap().text().unwrap();
        let res_json: Res1 = serde_json::from_str(&resp).unwrap();
        Ok(res_json.token)
    }

    fn get_layer_id(
        image_name: &str,
        image_tag: &str,
        token: &str,
    ) -> std::result::Result<String, ()> {
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
            .unwrap()
            .text()
            .unwrap();
        let res_json: Res2 = serde_json::from_str(&resp).unwrap();
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
    ) -> std::result::Result<(), ()> {
        let url = format!(
            "https://registry-1.docker.io/v2/library/{}/blobs/{}",
            image_name, layer_id
        );
        let client = reqwest::blocking::Client::new();
        let mut resp = client.get(&url).bearer_auth(token).send().unwrap();

        let mut file = File::create(file_path).unwrap();
        copy(&mut resp, &mut file).unwrap();
        Ok(())
    }
}
