//! `orca init / use / ls / rm / clean`.

use anyhow::Context;
use orca::{EnvStore, LockFile, Workspace};
use orca_image::BaseImageRef;
use orca_image::external_image::{Downloader, ImageIndex, LayerBlobStore};
use orca_vcs::{CommitStore, CommitsData};

use super::{confirm, select_env};

/// `orca init <name> [--image <ref>] [--keep]`: create an environment
/// with its initial commit graph, pulling the base image if needed.
pub fn init(
    store: &mut EnvStore,
    name: String,
    image: Option<String>,
    keep: bool,
) -> anyhow::Result<()> {
    let base_ref = match &image {
        None => BaseImageRef::Host,
        Some(reference) => {
            let root = orca::orca_root();
            let images_dir = root.join("images");
            let mut index = ImageIndex::load(&images_dir)?;
            let digest = match index.resolve(reference) {
                Ok(manifest) => manifest.digest,
                Err(_) => {
                    println!("pulling {reference}...");
                    let blobs = LayerBlobStore::new(&images_dir.join("layers").join("sha256"));
                    let manifest = Downloader::pull(reference, &blobs)
                        .with_context(|| format!("failed to pull {reference}"))?;
                    let digest = manifest.digest;
                    index.insert(manifest);
                    index.save()?;
                    digest
                }
            };
            BaseImageRef::External {
                image_digest: digest,
            }
        }
    };

    let (uuid, env_path) = {
        let env = store.create(name.clone(), base_ref)?;
        CommitStore::new(&env.env_path()).save(&CommitsData::new())?;
        (env.uuid, env.env_path())
    };
    if !keep {
        store.set_current(&uuid)?;
    }
    store.save()?;
    println!("created environment {name} ({uuid})");
    let _ = env_path; // created above; nothing further to report
    Ok(())
}

/// `orca use <name-or-uuid>`.
pub fn use_env(store: &mut EnvStore, target: &str) -> anyhow::Result<()> {
    let uuid = store.resolve(target)?.uuid;
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

/// `orca rm <name-or-uuid> [--yes]`: refuses while a container runs.
pub fn rm(store: &mut EnvStore, target: &str, yes: bool) -> anyhow::Result<()> {
    let env = store.resolve(target)?;
    let uuid = env.uuid;
    let name = env.name.clone();
    if let Some(pid) = LockFile::check(&env.lock_path())? {
        anyhow::bail!("a container is running in {name} (pid {pid}); stop it first");
    }
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
    Workspace::open(env)?.clean()?;
    println!("discarded uncommitted changes");
    Ok(())
}
