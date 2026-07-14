//! `orca init / use / ls / rm / clean`.

use anyhow::Context;
use orca::{BaseImageRef, EnvStore, Orca};

use super::{confirm, select_env};

/// `orca init <name> [--image <ref>] [--keep]`: create an environment,
/// pulling the base image if needed.
pub fn init(
    orca: &Orca,
    store: &mut EnvStore,
    name: String,
    image: Option<String>,
    keep: bool,
) -> anyhow::Result<()> {
    let base_ref = match &image {
        None => BaseImageRef::Host,
        Some(reference) => {
            let images = orca.external_images();
            let digest = match images.find(reference)? {
                Some(digest) => digest,
                None => {
                    println!("pulling {reference}...");
                    let manifest = images
                        .pull(reference)
                        .with_context(|| format!("failed to pull {reference}"))?;
                    manifest.digest
                }
            };
            BaseImageRef::External {
                image_digest: digest,
            }
        }
    };

    let uuid = store.create(name.clone(), base_ref)?.uuid;
    if !keep {
        store.set_current(&uuid)?;
    }
    store.save()?;
    println!("created environment {name} ({uuid})");
    Ok(())
}

/// `orca use <name-or-uuid>`.
pub fn use_env(store: &mut EnvStore, target: &str) -> anyhow::Result<()> {
    let uuid = store.env(Some(target))?.uuid;
    store.set_current(&uuid)?;
    store.save()?;
    println!("switched to {target}");
    Ok(())
}

/// `orca ls`.
pub fn ls(store: &EnvStore) -> anyhow::Result<()> {
    let current = store.current_uuid();
    println!("{:1} {:<20} {:<36} {:<10} CREATED", "", "NAME", "UUID", "BASE");
    for env in store.list() {
        let marker = if Some(env.uuid) == current { "*" } else { " " };
        let base = match &env.base_ref {
            BaseImageRef::Host => "host".to_string(),
            BaseImageRef::External { image_digest } => {
                format!("image:{}", &image_digest.to_string()[..12])
            }
        };
        println!(
            "{} {:<20} {:<36} {:<10} {}",
            marker,
            env.name,
            env.uuid,
            base,
            env.created_at.format("%Y-%m-%d %H:%M")
        );
    }
    Ok(())
}

/// `orca rm <name-or-uuid> [--yes]` (`delete` refuses while a container
/// runs).
pub fn rm(store: &mut EnvStore, target: &str, yes: bool) -> anyhow::Result<()> {
    let env = store.env(Some(target))?;
    let uuid = env.uuid;
    let name = env.name.clone();
    if !yes && !confirm(&format!("delete environment {name} and all its history?"))? {
        println!("aborted");
        return Ok(());
    }
    store.delete(&uuid)?;
    store.save()?;
    println!("deleted {name}");
    Ok(())
}

/// `orca clean`: discard the upper layer of the selected environment.
pub fn clean(store: &EnvStore, cli_env: &Option<String>) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    env.image()?.clean()?;
    println!("discarded uncommitted changes");
    Ok(())
}
