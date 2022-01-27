extern crate anyhow;
extern crate flate2;
extern crate reqwest;
extern crate rm_rf;
extern crate serde;
extern crate tar;

use anyhow::{bail, Context, Result};
use flate2::bufread::MultiGzDecoder;
use reqwest::blocking::{Client, Response};
use reqwest::{header, IntoUrl};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::fs::create_dir_all;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use tar::Archive;

#[derive(Serialize, Deserialize)]
struct Token {
    token: String,
    access_token: String,
    expires_in: u32,
    issued_at: String,
}

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Layer {
    mediaType: String,
    size: usize,
    digest: String,
}

#[derive(Serialize, Deserialize)]
#[allow(non_snake_case)]
struct Manifest {
    schemaVersion: usize,
    mediaType: String,
    config: Layer,
    layers: Vec<Layer>,
}

#[derive(Serialize, Deserialize)]
struct Config {}

pub struct ImageDownloader {
    image_name: String,
    image_tag: String,
    store_path: PathBuf,
    workdir_prefix: PathBuf,
}

impl ImageDownloader {
    pub fn new<S, T, U, V>(image_name: S, image_tag: T, store_path: U, workdir_prefix: V) -> Self
    where
        S: ToString + AsRef<str>,
        T: ToString,
        U: AsRef<Path>,
        V: AsRef<Path>,
    {
        let image_name = if !image_name.as_ref().contains('/') {
            format!("library/{}", image_name.as_ref())
        } else {
            image_name.to_string()
        };
        Self {
            image_name,
            image_tag: image_tag.to_string(),
            store_path: store_path.as_ref().to_path_buf(),
            workdir_prefix: workdir_prefix.as_ref().to_path_buf(),
        }
    }

    pub fn download_from_dockerhub(&self) -> Result<Vec<PathBuf>> {
        let token_url = format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
            self.image_name
        );
        let token = get_bearer_token(&token_url)?;

        let manifest_url = format!(
            "https://registry-1.docker.io/v2/{}/manifests/{}",
            self.image_name, self.image_tag
        );
        let layer_ids = get_layer_ids(&manifest_url, &token)?;

        create_dir_all(&self.workdir_prefix).with_context(|| {
            format!(
                "Failed to create dir: '{}'",
                &self.workdir_prefix.as_path().display()
            )
        })?;
        create_dir_all(&self.store_path).with_context(|| {
            format!(
                "Failed to create dir: '{}'",
                &self.store_path.as_path().display()
            )
        })?;

        let mut layers: Vec<PathBuf> = Vec::new();
        for (i, layer_id) in layer_ids.iter().enumerate() {
            let dir_name = format!("image_{}", i);
            let store_path = self.workdir_prefix.clone().join(dir_name);
            let image_url = format!(
                "https://registry-1.docker.io/v2/{}/blobs/{}",
                self.image_name, layer_id
            );
            let layer_tar_gz = download_layer_tarball(&image_url, &token)?;
            extract(layer_tar_gz, &store_path)?;
            layers.push(store_path);
        }
        Ok(layers)
    }
}

fn get_bearer_token<T: IntoUrl>(url: T) -> Result<String> {
    let client = Client::new();
    let res = client
        .get(url)
        .send()
        .context("Failed to get bearer token")?
        .text()
        .context("Failed to convert response to string")?;
    let res_token: std::result::Result<Token, serde_json::Error> = serde_json::from_str(&res);
    if res_token.is_err() {
        let errors: serde_json::Value =
            serde_json::from_str(&res).context("Failed to convert errors into json")?;
        bail!("token returns errors:\n{}", errors)
    }
    Ok(res_token.unwrap().token)
}

fn get_layer_ids<S: IntoUrl, T: Display>(url: S, token: T) -> Result<Vec<String>> {
    let client = Client::new();
    let res = client
        .get(url)
        .header(
            header::ACCEPT,
            "application/vnd.docker.distribution.manifest.v2+json",
        )
        .bearer_auth(token)
        .send()
        .context("Failed to get image id")?
        .text()
        .context("Failed to convert resopnse to string")?;

    let res_manifest: std::result::Result<Manifest, serde_json::Error> = serde_json::from_str(&res);
    if res_manifest.is_err() {
        let errors: serde_json::Value =
            serde_json::from_str(&res).context("Failed to convert errors into json")?;
        bail!("manifest returns errors:\n{}", errors)
    }
    let layers_ids: Vec<String> = res_manifest
        .unwrap()
        .layers
        .into_iter()
        .map(|layer| layer.digest)
        .collect();
    Ok(layers_ids)
}

#[allow(dead_code)]
fn get_config<S: Display, T: IntoUrl>(token: S, url: T) -> Result<Config> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .unwrap()
        .text()
        .unwrap();
    let _config: serde_json::Value = serde_json::from_str(&resp).unwrap();
    // let config = parse(config);
    Ok(Config {})
}

fn download_layer_tarball<S, T>(url: S, token: T) -> Result<Response>
where
    S: IntoUrl,
    T: Display,
{
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .context("Failed to donwload image")?;

    Ok(resp)
}

fn extract<S, T>(tar_gz: S, dest: T) -> Result<()>
where
    S: Read,
    T: AsRef<Path>,
{
    let reader = BufReader::new(tar_gz);
    let tar = MultiGzDecoder::new(reader);
    let mut archive = Archive::new(tar);
    archive
        .unpack(dest.as_ref())
        .context("Failed to unpack tarball")?;
    Ok(())
}
