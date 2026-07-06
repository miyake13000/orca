//! `orca image pull / ls / rm`.

use std::path::Path;

use anyhow::Context;
use orca_image::external_image::{Downloader, ImageIndex, LayerBlobStore};

/// `orca image pull <reference>`.
pub fn pull(root: &Path, reference: &str) -> anyhow::Result<()> {
    let images_dir = root.join("images");
    let mut index = ImageIndex::load(&images_dir)?;
    if index.resolve(reference).is_ok() {
        println!("{reference} is already pulled");
        return Ok(());
    }
    let blobs = LayerBlobStore::new(&images_dir.join("layers").join("sha256"));
    println!("pulling {reference}...");
    let manifest =
        Downloader::pull(reference, &blobs).with_context(|| format!("failed to pull {reference}"))?;
    println!(
        "pulled {}/{}:{} ({} layer(s))",
        manifest.registry,
        manifest.repository,
        manifest.tag,
        manifest.layer_digests.len()
    );
    index.insert(manifest);
    index.save()?;
    Ok(())
}

/// `orca image ls`.
pub fn ls(root: &Path) -> anyhow::Result<()> {
    let index = ImageIndex::load(&root.join("images"))?;
    println!(
        "{:<40} {:<15} {:<14} PULLED",
        "REPOSITORY", "TAG", "DIGEST"
    );
    for image in index.list() {
        let repo = if image.registry == "docker.io" {
            image.repository.clone()
        } else {
            format!("{}/{}", image.registry, image.repository)
        };
        println!(
            "{:<40} {:<15} {:<14} {}",
            repo,
            image.tag,
            &image.digest.to_string()[..12],
            image.pulled_at.format("%Y-%m-%d %H:%M")
        );
    }
    Ok(())
}

/// `orca image rm <reference>`: drop the record, then prune blobs no
/// image references anymore (layers are shared, so deletion is
/// reference-set based).
pub fn rm(root: &Path, reference: &str) -> anyhow::Result<()> {
    let images_dir = root.join("images");
    let mut index = ImageIndex::load(&images_dir)?;
    index.remove(reference)?;
    index.save()?;
    let blobs = LayerBlobStore::new(&images_dir.join("layers").join("sha256"));
    blobs.gc(&index.referenced_layers())?;
    println!("removed {reference}");
    Ok(())
}
