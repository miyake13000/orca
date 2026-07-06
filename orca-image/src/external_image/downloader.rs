//! [`Downloader`]: pulls images from OCI registries.
//!
//! Speaks the OCI Distribution API (Docker Registry HTTP API v2):
//!
//! - References are `[registry/]repo[:tag]`; the registry defaults to
//!   `docker.io` (whose API endpoint is `registry-1.docker.io`).
//! - Anonymous auth uses the standard token flow: a 401 carries a
//!   `WWW-Authenticate: Bearer realm=…,service=…` header naming the token
//!   endpoint. This works unchanged for docker.io / ghcr.io / quay.io
//!   public images; registries requiring login are future work.
//! - Multi-arch images (manifest list / OCI index) resolve to the
//!   `linux` + host-architecture entry.
//! - Layer tars are extracted with OCI whiteouts (`.wh.*`) converted to
//!   the OverlayFS native format (0:0 char device /
//!   `trusted.overlay.opaque` xattr) so mounted lowers interpret deletions
//!   correctly. This conversion requires root (mknod + trusted xattrs).

use std::io::Read;
use std::path::Path;

use chrono::Utc;
use serde::Deserialize;

use crate::whiteout;

use super::{ImageDigest, ImageManifest, LayerBlobStore, LayerDigest, Reference, parse_sha256};

/// Errors from pulling an image.
#[derive(Debug, thiserror::Error)]
pub enum PullError {
    /// HTTP transport failure.
    #[error("registry request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The registry answered with an unexpected status.
    #[error("registry returned {status} for {url}")]
    Status {
        /// HTTP status code.
        status: reqwest::StatusCode,
        /// Request URL.
        url: String,
    },
    /// Anonymous token authentication failed.
    #[error("registry authentication failed: {0}")]
    Auth(String),
    /// The manifest / config JSON had an unexpected shape.
    #[error("unexpected registry response: {0}")]
    Malformed(String),
    /// No manifest matches the host platform.
    #[error("no manifest for platform linux/{0}")]
    NoPlatform(String),
    /// A digest string could not be parsed.
    #[error(transparent)]
    Digest(#[from] super::IndexError),
    /// An unsupported layer media type was encountered.
    #[error("unsupported layer media type: {0}")]
    UnsupportedMediaType(String),
    /// Extraction or blob-store I/O failed.
    #[error("failed to store layer: {0}")]
    Io(#[from] std::io::Error),
    /// Blob store operation failed.
    #[error(transparent)]
    BlobStore(#[from] super::blob_store::BlobStoreError),
}

const ACCEPT_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json, \
     application/vnd.docker.distribution.manifest.list.v2+json, \
     application/vnd.oci.image.manifest.v1+json, \
     application/vnd.oci.image.index.v1+json";

#[derive(Deserialize)]
struct ManifestIndex {
    manifests: Vec<IndexEntry>,
}

#[derive(Deserialize)]
struct IndexEntry {
    digest: String,
    #[serde(default)]
    platform: Option<Platform>,
}

#[derive(Deserialize)]
struct Platform {
    #[serde(default)]
    architecture: String,
    #[serde(default)]
    os: String,
}

#[derive(Deserialize)]
struct ManifestV2 {
    config: Descriptor,
    layers: Vec<Descriptor>,
}

#[derive(Deserialize)]
struct Descriptor {
    digest: String,
    #[serde(rename = "mediaType", default)]
    media_type: String,
}

#[derive(Deserialize)]
struct ConfigBlob {
    #[serde(default)]
    config: RuntimeConfig,
}

#[derive(Deserialize, Default)]
struct RuntimeConfig {
    #[serde(rename = "Entrypoint", default)]
    entrypoint: Option<Vec<String>>,
    #[serde(rename = "Cmd", default)]
    cmd: Option<Vec<String>>,
    #[serde(rename = "Env", default)]
    env: Option<Vec<String>>,
    #[serde(rename = "WorkingDir", default)]
    working_dir: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
}

/// Registry client for `orca image pull`.
pub struct Downloader;

impl Downloader {
    /// Pull `reference`, extract missing layer blobs into `blobs`, and
    /// return the resolved [`ImageManifest`] (registering it in the index
    /// is the caller's job).
    ///
    /// Side effects: network access and blob extraction (which creates
    /// device nodes and trusted xattrs, hence requires root).
    pub fn pull(reference: &str, blobs: &LayerBlobStore) -> Result<ImageManifest, PullError> {
        let r = Reference::parse(reference);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()?;
        let mut session = Session {
            client,
            token: None,
        };

        // Fetch the manifest, resolving a multi-arch index if needed.
        let url = manifest_url(&r, &r.tag);
        let resp = session.get(&url, Some(ACCEPT_MANIFEST), &r)?;
        let digest_header = header_digest(&resp);
        let media_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = resp.bytes()?;

        let (manifest, manifest_digest) = if media_type.contains("list") || media_type.contains("index") {
            let index: ManifestIndex = serde_json::from_slice(&body)
                .map_err(|e| PullError::Malformed(e.to_string()))?;
            let arch = host_architecture();
            let entry = index
                .manifests
                .iter()
                .find(|m| {
                    m.platform
                        .as_ref()
                        .is_some_and(|p| p.os == "linux" && p.architecture == arch)
                })
                .ok_or_else(|| PullError::NoPlatform(arch.to_string()))?;
            let url = manifest_url(&r, &entry.digest);
            let resp = session.get(&url, Some(ACCEPT_MANIFEST), &r)?;
            let digest = parse_sha256(&entry.digest)?;
            let body = resp.bytes()?;
            let manifest: ManifestV2 = serde_json::from_slice(&body)
                .map_err(|e| PullError::Malformed(e.to_string()))?;
            (manifest, digest)
        } else {
            let manifest: ManifestV2 = serde_json::from_slice(&body)
                .map_err(|e| PullError::Malformed(e.to_string()))?;
            let digest = match digest_header {
                Some(d) => parse_sha256(&d)?,
                None => sha256_of(&body),
            };
            (manifest, digest)
        };

        // Fetch the config blob for entrypoint/cmd/env/working_dir.
        let config_url = blob_url(&r, &manifest.config.digest);
        let config: ConfigBlob = serde_json::from_slice(
            &session.get(&config_url, None, &r)?.bytes()?,
        )
        .map_err(|e| PullError::Malformed(e.to_string()))?;

        // Download and extract missing layers.
        let mut layer_digests = Vec::new();
        for layer in &manifest.layers {
            let digest = LayerDigest(parse_sha256(&layer.digest)?);
            layer_digests.push(digest);
            if blobs.contains(&digest) {
                continue;
            }
            let resp = session.get(&blob_url(&r, &layer.digest), None, &r)?;
            extract_layer(resp, &layer.media_type, blobs, &digest)?;
        }

        let rc = config.config;
        Ok(ImageManifest {
            digest: ImageDigest(manifest_digest),
            registry: r.registry,
            repository: r.repository,
            tag: r.tag,
            layer_digests,
            entrypoint: rc.entrypoint.unwrap_or_default(),
            cmd: rc.cmd.unwrap_or_default(),
            env: rc.env.unwrap_or_default(),
            working_dir: rc.working_dir.unwrap_or_default().into(),
            pulled_at: Utc::now(),
        })
    }
}

/// HTTP session holding an optional bearer token.
struct Session {
    client: reqwest::blocking::Client,
    token: Option<String>,
}

impl Session {
    /// GET `url`; on a 401, run the anonymous token flow advertised in
    /// `WWW-Authenticate` and retry once.
    fn get(
        &mut self,
        url: &str,
        accept: Option<&str>,
        r: &Reference,
    ) -> Result<reqwest::blocking::Response, PullError> {
        for attempt in 0..2 {
            let mut req = self.client.get(url);
            if let Some(accept) = accept {
                req = req.header(reqwest::header::ACCEPT, accept);
            }
            if let Some(token) = &self.token {
                req = req.bearer_auth(token);
            }
            let resp = req.send()?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                let challenge = resp
                    .headers()
                    .get(reqwest::header::WWW_AUTHENTICATE)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| PullError::Auth("401 without WWW-Authenticate".into()))?
                    .to_string();
                self.token = Some(self.fetch_token(&challenge, r)?);
                continue;
            }
            if !resp.status().is_success() {
                return Err(PullError::Status {
                    status: resp.status(),
                    url: url.to_string(),
                });
            }
            return Ok(resp);
        }
        Err(PullError::Auth("authentication retry failed".into()))
    }

    /// Anonymous token flow: parse `Bearer realm=…,service=…` and GET
    /// `realm?service=…&scope=repository:<repo>:pull`.
    fn fetch_token(&self, challenge: &str, r: &Reference) -> Result<String, PullError> {
        let params = parse_challenge(challenge);
        let realm = params
            .iter()
            .find(|(k, _)| k == "realm")
            .map(|(_, v)| v.clone())
            .ok_or_else(|| PullError::Auth(format!("no realm in challenge: {challenge}")))?;
        let mut req = self.client.get(&realm).query(&[(
            "scope",
            format!("repository:{}:pull", r.repository),
        )]);
        if let Some((_, service)) = params.iter().find(|(k, _)| k == "service") {
            req = req.query(&[("service", service)]);
        }
        let resp: TokenResponse = req.send()?.json()?;
        resp.token
            .or(resp.access_token)
            .ok_or_else(|| PullError::Auth("token endpoint returned no token".into()))
    }
}

/// Parse a `Bearer k="v",k2="v2"` challenge into key/value pairs.
fn parse_challenge(challenge: &str) -> Vec<(String, String)> {
    let rest = challenge.strip_prefix("Bearer ").unwrap_or(challenge);
    rest.split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        })
        .collect()
}

/// API host for a registry (docker.io's API lives on registry-1.docker.io).
fn api_host(registry: &str) -> &str {
    match registry {
        "docker.io" => "registry-1.docker.io",
        other => other,
    }
}

fn manifest_url(r: &Reference, tag_or_digest: &str) -> String {
    format!(
        "https://{}/v2/{}/manifests/{}",
        api_host(&r.registry),
        r.repository,
        tag_or_digest
    )
}

fn blob_url(r: &Reference, digest: &str) -> String {
    format!(
        "https://{}/v2/{}/blobs/{}",
        api_host(&r.registry),
        r.repository,
        digest
    )
}

/// Map Rust's target arch to OCI platform architecture names.
fn host_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "arm",
        "riscv64" => "riscv64",
        other => other,
    }
}

fn header_digest(resp: &reqwest::blocking::Response) -> Option<String> {
    resp.headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn sha256_of(data: &[u8]) -> orca_hash::Hash {
    use orca_hash::Hasher as _;
    let mut h = orca_hash::Sha256Hasher::new();
    h.update(data);
    h.finalize()
}

/// Extract a layer blob into the store, converting OCI whiteouts to the
/// OverlayFS native format. Extraction happens in a sibling temp directory
/// so the final `save` is an atomic same-filesystem rename.
fn extract_layer(
    resp: reqwest::blocking::Response,
    media_type: &str,
    blobs: &LayerBlobStore,
    digest: &LayerDigest,
) -> Result<(), PullError> {
    let reader: Box<dyn Read> = if media_type.contains("gzip") || media_type.is_empty() {
        Box::new(flate2::read::GzDecoder::new(resp))
    } else if media_type.contains("tar") && !media_type.contains("zstd") {
        Box::new(resp)
    } else {
        return Err(PullError::UnsupportedMediaType(media_type.to_string()));
    };

    let staging = blobs.blob_path(digest).with_extension("tmp");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    let result = unpack_with_whiteout_conversion(reader, &staging);
    match result {
        Ok(()) => {
            blobs.save(digest, &staging)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// Unpack a tar stream into `dest`, converting `.wh.<name>` entries into
/// 0:0 char-device whiteouts and `.wh..wh..opq` entries into the opaque
/// xattr on their parent directory.
fn unpack_with_whiteout_conversion(reader: impl Read, dest: &Path) -> Result<(), PullError> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(true);
    archive.set_preserve_ownerships(true);
    archive.set_unpack_xattrs(true);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string);
        if let Some(name) = file_name {
            if name == ".wh..wh..opq" {
                let parent = path.parent().unwrap_or_else(|| Path::new(""));
                let dir = dest.join(parent);
                std::fs::create_dir_all(&dir)?;
                whiteout::set_opaque(&dir)?;
                continue;
            }
            if let Some(stripped) = name.strip_prefix(".wh.") {
                let parent = path.parent().unwrap_or_else(|| Path::new(""));
                let dir = dest.join(parent);
                std::fs::create_dir_all(&dir)?;
                whiteout::make_whiteout(&dir.join(stripped))?;
                continue;
            }
        }
        entry.unpack_in(dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_parsing() {
        let params = parse_challenge(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io""#,
        );
        assert!(params.contains(&(
            "realm".to_string(),
            "https://auth.docker.io/token".to_string()
        )));
        assert!(params.contains(&(
            "service".to_string(),
            "registry.docker.io".to_string()
        )));
    }

    #[test]
    fn docker_io_maps_to_registry_1() {
        let r = Reference::parse("ubuntu:24.04");
        assert_eq!(
            manifest_url(&r, "24.04"),
            "https://registry-1.docker.io/v2/library/ubuntu/manifests/24.04"
        );
    }
}
