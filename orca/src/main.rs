//! orca CLI entry point: argument parsing (clap) and dispatch.
//!
//! The binary is a thin adapter over the `orca` library: it parses
//! arguments, resolves the target environment, runs prompts, formats
//! output, and maps typed errors to messages and exit codes. All state
//! operations live in the library.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod cli;

/// Global CLI definition.
#[derive(Parser)]
#[command(
    name = "orca",
    version,
    about = "Container environments with Git-like version control"
)]
struct Cli {
    /// Data root (config.toml, envs, images and run). Relative to the invocation cwd.
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    /// Target environment (name or uuid); overrides ORCA_ENV and the
    /// current environment.
    #[arg(long, short = 'e', global = true)]
    env: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new environment (host rootfs based by default)
    Init {
        /// Environment name
        name: String,
        /// Explicitly use the host rootfs as base (default)
        #[arg(long, conflicts_with = "image")]
        host: bool,
        /// Use a Docker/OCI image as base (pulled automatically)
        #[arg(long, short = 'i')]
        image: Option<String>,
        /// Do not switch the current environment to the new one
        #[arg(long, short = 'k')]
        keep: bool,
    },
    /// Switch the current environment
    Use {
        /// Environment name or uuid
        target: String,
    },
    /// List environments
    Ls,
    /// Delete an environment entirely
    Rm {
        /// Environment name or uuid (explicit, to avoid accidents)
        target: String,
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Discard uncommitted changes (destroy the upper layer)
    Clean,
    /// Enter the container, or run a command inside it
    Run {
        /// Isolate PID namespace; use --pid=false to share.
        #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
        pid: Option<bool>,
        /// Share PID namespace (alias for --pid=false).
        #[arg(long, conflicts_with = "pid")]
        no_pid: bool,
        /// Isolate UTS namespace; use --uts=false to share.
        #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
        uts: Option<bool>,
        /// Share UTS namespace.
        #[arg(long, conflicts_with = "uts")]
        no_uts: bool,
        /// Isolate IPC namespace; use --ipc=false to share.
        #[arg(long, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
        ipc: Option<bool>,
        /// Share IPC namespace.
        #[arg(long, conflicts_with = "ipc")]
        no_ipc: bool,
        /// Isolate network namespace; use --network=false to share.
        #[arg(long, short = 'n', num_args = 0..=1, default_missing_value = "true", require_equals = true)]
        network: Option<bool>,
        /// Literal environment override (repeatable KEY=VALUE).
        #[arg(long = "set-env", value_name = "KEY=VALUE", value_parser = parse_env)]
        variables: Vec<(String, String)>,
        /// Absolute working directory inside the container.
        #[arg(long)]
        working_dir: Option<PathBuf>,
        /// Run as this user inside the container (uid or name).
        /// Overrides config and the image USER declaration
        #[arg(long, short = 'u')]
        user: Option<String>,
        /// Run with this group inside the container (gid or name)
        #[arg(long, short = 'g')]
        group: Option<String>,
        /// Command and arguments (defaults to the image command)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
    /// Commit the current changes
    Commit {
        /// Commit message
        #[arg(short = 'm', long = "message")]
        message: String,
    },
    /// Show changes (uncommitted, or of a commit)
    Diff {
        /// Append an exclusion or ! exception (repeatable).
        #[arg(long, allow_hyphen_values = true)]
        blacklist: Vec<String>,
        /// Commit/branch A (omit for uncommitted changes)
        a: Option<String>,
        /// Commit/branch B to compare A against (default: the base)
        b: Option<String>,
    },
    /// Show commit history
    Log,
    /// Create, delete or list branches
    Branch {
        /// Branch name to create (from current HEAD)
        name: Option<String>,
        /// Delete the named branch
        #[arg(short = 'd', long = "delete", conflicts_with_all = ["name", "all"])]
        delete: Option<String>,
        /// List all branches
        #[arg(short = 'a', long = "all", conflicts_with = "name")]
        all: bool,
    },
    /// Switch to a branch or commit (clean state required)
    Checkout {
        /// Branch name, commit hash (prefix ok), or ROOT
        #[arg(required_unless_present = "branch")]
        target: Option<String>,
        /// Create a branch and switch to it
        #[arg(short = 'b')]
        branch: Option<String>,
    },
    /// Hard reset the current branch to a commit
    Reset {
        /// Commit hash (prefix ok) or ROOT
        commit: String,
    },
    /// Rebase target branch onto newbase
    Rebase {
        /// The branch to graft onto
        newbase: String,
        /// The branch whose commits are replayed
        target: String,
    },
    /// Merge a branch (not implemented; use rebase)
    Merge {
        /// Branch to merge
        branch: String,
    },
    /// Remove unreachable commits and layers
    Gc,
    /// Apply the environment's changes to the host (host-based only)
    Apply {
        /// Append an exclusion or ! exception (repeatable).
        #[arg(long, allow_hyphen_values = true)]
        blacklist: Vec<String>,
        /// Apply committed changes only (exclude uncommitted)
        #[arg(long)]
        no_upper: bool,
        /// Show what would change without applying
        #[arg(long, short = 'n')]
        dry_run: bool,
        /// Skip the confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Manage cached images
    Image {
        #[command(subcommand)]
        command: ImageCommand,
    },
}

#[derive(Subcommand)]
enum ImageCommand {
    /// Pull an image from a registry
    Pull {
        /// Image reference, e.g. ubuntu:24.04 or ghcr.io/owner/repo:v1
        reference: String,
    },
    /// List cached images
    Ls,
    /// Remove a cached image (unreferenced layers are pruned)
    Rm {
        /// Image reference
        reference: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let exit = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    };
    std::process::exit(exit);
}

/// Dispatch to the command handlers. Returns the process exit code
/// (`orca run` propagates the container's).
fn run(cli: Cli) -> anyhow::Result<i32> {
    let root = cli.root.unwrap_or_else(orca::orca_root);
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()?.join(root)
    };
    let orca = orca::Orca::new(root);
    let config = orca.config()?;
    let mut store = orca.envs()?;

    match cli.command {
        Command::Init {
            name,
            host: _,
            image,
            keep,
        } => cli::env_cmd::init(&orca, &mut store, name, image, keep),
        Command::Use { target } => cli::env_cmd::use_env(&mut store, &target),
        Command::Ls => cli::env_cmd::ls(&store),
        Command::Rm { target, yes } => cli::env_cmd::rm(&mut store, &target, yes),
        Command::Clean => cli::env_cmd::clean(&store, &cli.env),
        Command::Run {
            pid,
            no_pid,
            uts,
            no_uts,
            ipc,
            no_ipc,
            network,
            user,
            group,
            variables,
            working_dir,
            cmd,
        } => {
            let env = cli::select_env(&store, &cli.env)?;
            let args = orca::Settings {
                cmd: (!cmd.is_empty()).then_some(cmd),
                env: variables.into_iter().collect(),
                working_dir,
                user,
                group,
                namespaces: orca::NamespaceSettings {
                    pid: if no_pid { Some(false) } else { pid },
                    uts: if no_uts { Some(false) } else { uts },
                    ipc: if no_ipc { Some(false) } else { ipc },
                    network,
                },
                ..Default::default()
            };
            return Ok(env.image()?.run(config.resolve(&env.name, &args)?)?);
        }
        Command::Commit { message } => cli::vcs_cmd::commit(&store, &cli.env, &message),
        Command::Diff { a, b, blacklist } => {
            let env = cli::select_env(&store, &cli.env)?;
            let settings = config.resolve(
                &env.name,
                &orca::Settings {
                    blacklist,
                    ..Default::default()
                },
            )?;
            cli::vcs_cmd::diff(
                &store,
                &cli.env,
                a.as_deref(),
                b.as_deref(),
                &settings.blacklist,
            )
        }
        Command::Log => cli::vcs_cmd::log(&store, &cli.env),
        Command::Branch { name, delete, all } => {
            cli::vcs_cmd::branch(&store, &cli.env, name, delete, all)
        }
        Command::Checkout { target, branch } => {
            cli::vcs_cmd::checkout(&store, &cli.env, target, branch)
        }
        Command::Reset { commit } => cli::vcs_cmd::reset(&store, &cli.env, &commit),
        Command::Rebase { newbase, target } => {
            cli::vcs_cmd::rebase(&store, &cli.env, &newbase, &target)
        }
        Command::Merge { branch } => cli::vcs_cmd::merge(&store, &cli.env, &branch),
        Command::Gc => cli::vcs_cmd::gc(&store, &cli.env),
        Command::Apply {
            blacklist,
            no_upper,
            dry_run,
            yes,
        } => {
            let env = cli::select_env(&store, &cli.env)?;
            let settings = config.resolve(
                &env.name,
                &orca::Settings {
                    blacklist,
                    ..Default::default()
                },
            )?;
            cli::apply_cmd::apply(
                &store,
                &cli.env,
                no_upper,
                dry_run,
                yes,
                &settings.blacklist,
            )
        }
        Command::Image { command } => match command {
            ImageCommand::Pull { reference } => cli::image_cmd::pull(&orca, &reference),
            ImageCommand::Ls => cli::image_cmd::ls(&orca),
            ImageCommand::Rm { reference } => cli::image_cmd::rm(&orca, &reference),
        },
    }?;
    Ok(0)
}

/// Parse one literal environment assignment; later occurrences replace earlier ones.
fn parse_env(value: &str) -> Result<(String, String), String> {
    let (key, value) = value.split_once('=').ok_or("expected KEY=VALUE")?;
    if key.is_empty() || key.contains('\0') || value.contains('\0') {
        return Err("invalid environment assignment".into());
    }
    Ok((key.into(), value.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boolean_overrides_and_command_arguments() {
        let cli = Cli::try_parse_from([
            "orca",
            "--root",
            "data",
            "run",
            "--network=false",
            "--pid",
            "--set-env",
            "X=a=b",
            "bash",
            "-l",
        ])
        .unwrap();
        match cli.command {
            Command::Run {
                pid,
                network,
                variables,
                cmd,
                ..
            } => {
                assert_eq!(pid, Some(true));
                assert_eq!(network, Some(false));
                assert_eq!(variables, vec![("X".into(), "a=b".into())]);
                assert_eq!(cmd, ["bash", "-l"]);
            }
            _ => panic!("wrong command"),
        }
        assert!(Cli::try_parse_from(["orca", "run", "--pid", "--no-pid"]).is_err());
    }
}
