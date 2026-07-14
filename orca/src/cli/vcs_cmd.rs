//! `orca commit / diff / log / branch / checkout / reset / rebase /
//! merge / gc`.

use orca::{EnvStore, Head};

use super::{format_change, select_env, short_hash};

/// `orca commit -m <message>`.
pub fn commit(store: &EnvStore, cli_env: &Option<String>, message: &str) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    let hash = env.image()?.commit(message)?;
    println!("committed {}", short_hash(&hash));
    Ok(())
}

/// `orca diff [A [B]]`.
pub fn diff(
    store: &EnvStore,
    cli_env: &Option<String>,
    a: Option<&str>,
    b: Option<&str>,
) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    let image = env.image()?;
    for change in image.diff(a, b)? {
        println!("{}", format_change(&change));
    }
    Ok(())
}

/// `orca log`: first-parent history of HEAD with branch decorations.
pub fn log(store: &EnvStore, cli_env: &Option<String>) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    let image = env.image()?;
    let data = image.data();
    let head = data.head();
    for commit in image.log() {
        let mut decorations: Vec<String> = Vec::new();
        if head.commit_hash() == *commit.hash() {
            match head.branch_name() {
                Some(branch) => decorations.push(format!("HEAD -> {branch}")),
                None => decorations.push("HEAD".to_string()),
            }
        }
        for branch in data.branches() {
            if branch.commit_hash() == commit.hash() && Some(branch.name()) != head.branch_name()
            {
                decorations.push(branch.name().to_string());
            }
        }
        let deco = if decorations.is_empty() {
            String::new()
        } else {
            format!(" ({})", decorations.join(", "))
        };
        let message = if commit.is_root() && commit.message().is_empty() {
            "(initial commit)"
        } else {
            commit.message()
        };
        println!(
            "{} {} {}{}",
            short_hash(commit.hash()),
            commit.timestamp().format("%Y-%m-%d %H:%M:%S"),
            message,
            deco
        );
    }
    Ok(())
}

/// `orca branch [<name>] [-d <name>] [-a]`.
pub fn branch(
    store: &EnvStore,
    cli_env: &Option<String>,
    name: Option<String>,
    delete: Option<String>,
    all: bool,
) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    let mut image = env.image()?;
    if let Some(name) = delete {
        image.branch_delete(&name)?;
        println!("deleted branch {name}");
    } else if let Some(name) = name {
        image.branch_create(&name)?;
        println!("created branch {name}");
    } else {
        // -a or bare `orca branch`: list.
        let _ = all;
        let data = image.data();
        let current = data.head().branch_name();
        for branch in data.branches() {
            let marker = if Some(branch.name()) == current { "*" } else { " " };
            println!(
                "{} {} {}",
                marker,
                branch.name(),
                short_hash(branch.commit_hash())
            );
        }
    }
    Ok(())
}

/// `orca checkout <target>` / `orca checkout -b <name>`.
pub fn checkout(
    store: &EnvStore,
    cli_env: &Option<String>,
    target: Option<String>,
    new_branch: Option<String>,
) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    let mut image = env.image()?;
    if let Some(name) = new_branch {
        image.branch_create(&name)?;
        image.checkout(&name)?;
        println!("switched to new branch {name}");
        return Ok(());
    }
    let target = target.expect("clap enforces target xor -b");
    match image.checkout(&target)? {
        Head::Branch(name) => println!("switched to branch {name}"),
        Head::Detached(hash) => println!(
            "HEAD is now detached at {} (commits are disabled)",
            short_hash(&hash)
        ),
    }
    Ok(())
}

/// `orca reset <commit>`.
pub fn reset(store: &EnvStore, cli_env: &Option<String>, commit: &str) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    let hash = env.image()?.reset(commit)?;
    println!("reset to {}", short_hash(&hash));
    Ok(())
}

/// `orca rebase <newbase> <target>`.
pub fn rebase(
    store: &EnvStore,
    cli_env: &Option<String>,
    newbase: &str,
    target: &str,
) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    env.image()?.rebase(newbase, target)?;
    println!("rebased {target} onto {newbase}");
    Ok(())
}

/// `orca merge <branch>`: reserved; `Image::merge` always reports
/// the rebase alternative.
pub fn merge(store: &EnvStore, cli_env: &Option<String>, branch: &str) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    env.image()?.merge(branch)?;
    Ok(())
}

/// `orca gc`.
pub fn gc(store: &EnvStore, cli_env: &Option<String>) -> anyhow::Result<()> {
    let env = select_env(store, cli_env)?;
    let dead = env.image()?.gc()?;
    println!("removed {} unreachable commit(s)", dead.len());
    Ok(())
}
