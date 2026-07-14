//! `orca image pull / ls / rm`.

use anyhow::Context;
use orca::Orca;

/// `orca image pull <reference>`.
pub fn pull(orca: &Orca, reference: &str) -> anyhow::Result<()> {
    let images = orca.external_images();
    if images.find(reference)?.is_some() {
        println!("{reference} is already pulled");
        return Ok(());
    }
    println!("pulling {reference}...");
    let manifest = images
        .pull(reference)
        .with_context(|| format!("failed to pull {reference}"))?;
    println!(
        "pulled {}/{}:{} ({} layer(s))",
        manifest.registry,
        manifest.repository,
        manifest.tag,
        manifest.layer_digests.len()
    );
    Ok(())
}

/// `orca image ls`.
pub fn ls(orca: &Orca) -> anyhow::Result<()> {
    println!(
        "{:<40} {:<15} {:<14} PULLED",
        "REPOSITORY", "TAG", "DIGEST"
    );
    for image in orca.external_images().list()? {
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

/// `orca image rm <reference>` (unreferenced blobs are pruned by the
/// store).
pub fn rm(orca: &Orca, reference: &str) -> anyhow::Result<()> {
    orca.external_images().remove(reference)?;
    println!("removed {reference}");
    Ok(())
}
