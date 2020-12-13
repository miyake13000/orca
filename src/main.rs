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
struct CurlRes1 {
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
struct CurlRes2 {
    schemaVersion: usize,
    mediaType: String,
    config: Config,
    layers: Vec<Config>,
}

fn main() {
    let dest_name = "alpine";
    let dest_ver = "3.10";

    let mut path = PathBuf::new();
    let home_dir = home_dir().unwrap();
    path.push(home_dir);
    path.push(".local/orca/containers");
    path.push(dest_name);
    path.push(dest_ver);
    println!("path:{}", path.to_str().unwrap());

    if Path::new(path.as_path()).exists() {
        println!("Container image has already existed");
        return;
    }

    let url = format!("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/{}:pull", dest_name);
    let client = reqwest::blocking::Client::new();
    let resp = client.get(&url)
        .send()
        .unwrap()
        .text()
        .unwrap();
    let res_json: CurlRes1 = serde_json::from_str(&resp).unwrap();
    let token = &res_json.token;
    println!("Found image");

    let url = format!("https://registry-1.docker.io/v2/library/{}/manifests/{}", dest_name, dest_ver);
    let client = reqwest::blocking::Client::new();
    let resp = client.get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.docker.distribution.manifest.v2+json")
        .bearer_auth(token)
        .send()
        .unwrap()
        .text()
        .unwrap();
    let res_json: CurlRes2 = serde_json::from_str(&resp).unwrap();
    let layer_id = &res_json.layers[0].digest;

    println!("Downloading image...");
    let url = format!("https://registry-1.docker.io/v2/library/{}/blobs/{}", dest_name, layer_id);
    let client = reqwest::blocking::Client::new();
    let mut resp = client.get(&url)
        .bearer_auth(token)
        .send()
        .unwrap();

    create_dir_all(path.as_path()).expect("create directory");
    path.push("image.tar.gz");
    println!("path:{}", path.to_str().unwrap());
    let mut file = File::create(path.as_path()).expect("file create");
    io::copy(&mut resp, &mut file).expect("copy");

    println!("Extracting image");
    let tar_gz = File::open(path.as_path()).expect("file open");
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    path.pop();
    path.push("rootfs");
    println!("path:{}", path.to_str().unwrap());
    archive.unpack(path.as_path()).expect("unpack");

    return;
}

