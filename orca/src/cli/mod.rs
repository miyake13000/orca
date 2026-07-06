//! CLI command handlers (thin adapters over the `orca` library).

pub mod apply_cmd;
pub mod env_cmd;
pub mod image_cmd;
pub mod run;
pub mod vcs_cmd;

use anyhow::Context;
use orca::{Env, EnvStore};

/// Resolve the target environment: `--env` → `ORCA_ENV` → current.
pub fn select_env<'a>(
    store: &'a EnvStore,
    cli_env: &Option<String>,
) -> anyhow::Result<&'a Env> {
    if let Some(target) = cli_env {
        return Ok(store.resolve(target)?);
    }
    if let Ok(target) = std::env::var("ORCA_ENV")
        && !target.is_empty()
    {
        return store
            .resolve(&target)
            .with_context(|| format!("ORCA_ENV={target}"));
    }
    Ok(store.current()?)
}

/// Ask a yes/no question on the terminal; returns true on `y`/`yes`.
pub fn confirm(question: &str) -> anyhow::Result<bool> {
    use std::io::Write;
    print!("{question} [y/N]: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

/// Short (12-char) form of a commit hash for display.
pub fn short_hash(hash: &orca_hash::Hash) -> String {
    orca_hash::to_hex(hash)[..12].to_string()
}

/// One-letter tag + path rendering for a change.
pub fn format_change(change: &orca_image::Change) -> String {
    let (tag, path) = match change {
        orca_image::Change::Create { path, .. } => ('A', path),
        orca_image::Change::Modify { path, .. } => ('M', path),
        orca_image::Change::Delete { path } => ('D', path),
    };
    format!("{tag} /{}", path.display())
}
