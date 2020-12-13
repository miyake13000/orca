// This program is created by nomlab in Okayama University
// https://github.com/miyake13000/crca

extern crate reqwest;
extern crate serde;
extern crate serde_json;
use serde::{Deserialize, Serialize};
use std::io;
use std::fs::{File, create_dir_all};
use std::path::{Path, PathBuf};
use flate2::read::GzDecoder;
use tar::Archive;
use dirs::home_dir;

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
    dest_name: String,
    dest_tag:  String,
    path: PathBuf,
}

impl Image {
    pub fn new(name: String, tag: String) -> Image {
        let mut path = PathBuf::new();
        let home_dir = home_dir().unwrap();
        path.push(home_dir);
        path.push(".local/orca/containers");
        path.push(&name);
        path.push(&tag);

        Image{dest_name: name, dest_tag: tag, path: path}
    }

    pub fn exist(&self) -> bool {
        Path::new(self.path.as_path()).exists()
    }

    pub fn get(&self) -> Result<(), ()> {
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

        create_dir_all(self.path.as_path()).expect("create directory");
        let path = format!("{}/image.tar.gz", self.path.to_str().unwrap());
        let mut file = File::create(path).expect("file create");
        io::copy(&mut resp, &mut file).expect("copy");
        Ok(())
    }

    pub fn extract(&self) -> Result<(), ()> {
        let path = format!("{}/image.tar.gz", self.path.to_str().unwrap());
        let tar_gz = File::open(path).expect("file open");
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        let path = format!("{}/rootfs", self.path.to_str().unwrap());
        archive.unpack(path).expect("unpack");
        Ok(())
    }
}

fn main() {
    let image = Image::new(String::from("debian"), String::from("latest"));
    if  image.exist() {
        println!("image has exist");
    } else {
        println!("Downloading container image...");
        image.get().unwrap();
        println!("Extracting container image...");
        image.extract().unwrap();
        println!("done");
    }
}

