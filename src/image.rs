// This file has below struct and impl
// ・Image : operate container image

extern crate reqwest;
extern crate serde;
extern crate serde_json;

use serde::{Deserialize, Serialize};
use std::io::{self, copy};
use std::fs::File;
use std::process::Command;

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

pub struct Image<'a, 'b> {
    pub dest_name: &'a str,
    pub dest_tag:  &'b str,
}

impl Image<'_, '_> {
    pub fn new<'a>(name: &'a str, tag: &'a str) -> Image<'a, 'a> {
        Image{
            dest_name: name,
            dest_tag: tag,
        }
    }

    pub fn get_token(&self) -> io::Result<String> {
        let url = format!("https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/{}:pull", self.dest_name);
        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url)
            .send()
            .unwrap()
            .text()
            .unwrap();
        let res_json: Res1 = serde_json::from_str(&resp).unwrap();
        Ok(res_json.token)
    }

    pub fn get_layer_id(&self, token: &str) -> io::Result<String> {
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
        Ok((&res_json.layers[0].digest).to_string())
    }

    #[allow(dead_code)]
    pub fn get_var(&self, token: &str, image_id: &str) -> io::Result<(String, String)> {
        let url = format!("https://registry-1.docker.io/v2/library/{}/blobs/{}", self.dest_name, image_id);

        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url)
            .bearer_auth(token)
            .send()
            .unwrap()
            .text()
            .unwrap();
        let _res_json: Res2 = serde_json::from_str(&resp).unwrap();
        Ok((String::from(""), String::from("")))
    }

    pub fn download(&self, token: &str, layer_id: &str, path: &str) -> io::Result<()> {
        let url = format!("https://registry-1.docker.io/v2/library/{}/blobs/{}", self.dest_name, layer_id);
        let client = reqwest::blocking::Client::new();
        let mut resp = client.get(&url)
            .bearer_auth(token)
            .send()
            .unwrap();

        let mut file = File::create(path).expect("file create");
        copy(&mut resp, &mut file).unwrap();
        Ok(())
    }

    pub fn extract(&self, path_src: &str, path_dest: &str) -> io::Result<()> {
        let _ = Command::new("tar")
                         .arg("-xzf")
                         .arg(path_src)
                         .arg("-C")
                         .arg(path_dest)
                         .output()
                         .expect("exec tar");
        Ok(())
    }
}
