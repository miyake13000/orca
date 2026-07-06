//! orca CLI entry point: argument parsing (clap) and dispatch.
//!
//! The binary is a thin adapter over the `orca` library: it parses
//! arguments, resolves the target environment, runs prompts, formats
//! output, and maps typed errors to messages and exit codes. All state
//! operations live in the library.

use clap::{Parser, Subcommand};

mod cli;

/// Global CLI definition.
#[derive(Parser)]
#[command(name = "orca", version, about = "Container environments with Git-like version control")]
struct Cli {
    /// Target environment (name or uuid); overrides ORCA_ENV and the
    /// current environment.
    #[arg(long, global = true)]
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
        #[arg(long)]
        image: Option<String>,
        /// Do not switch the current environment to the new one
        #[arg(long)]
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
        /// Share the host PID namespace
        #[arg(long)]
        no_pid: bool,
        /// Share the host UTS namespace
        #[arg(long)]
        no_uts: bool,
        /// Share the host IPC namespace
        #[arg(long)]
        no_ipc: bool,
        /// Isolate the network namespace (shared with host by default)
        #[arg(long)]
        network: bool,
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
        /// Apply committed changes only (exclude uncommitted)
        #[arg(long)]
        no_upper: bool,
        /// Show what would change without applying
        #[arg(long)]
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
    let root = orca::orca_root();
    let mut store = orca::EnvStore::load(&root.join("envs"))?;

    match cli.command {
        Command::Init {
            name,
            host: _,
            image,
            keep,
        } => cli::env_cmd::init(&mut store, name, image, keep),
        Command::Use { target } => cli::env_cmd::use_env(&mut store, &target),
        Command::Ls => cli::env_cmd::ls(&store),
        Command::Rm { target, yes } => cli::env_cmd::rm(&mut store, &target, yes),
        Command::Clean => cli::env_cmd::clean(&store, &cli.env),
        Command::Run {
            no_pid,
            no_uts,
            no_ipc,
            network,
            cmd,
        } => {
            let env = cli::select_env(&store, &cli.env)?;
            return cli::run::run(
                env,
                cli::run::RunOpts {
                    no_pid,
                    no_uts,
                    no_ipc,
                    network,
                    cmd,
                },
            );
        }
        Command::Commit { message } => cli::vcs_cmd::commit(&store, &cli.env, &message),
        Command::Diff { a, b } => cli::vcs_cmd::diff(&store, &cli.env, a.as_deref(), b.as_deref()),
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
            no_upper,
            dry_run,
            yes,
        } => cli::apply_cmd::apply(&store, &cli.env, no_upper, dry_run, yes),
        Command::Image { command } => match command {
            ImageCommand::Pull { reference } => cli::image_cmd::pull(&root, &reference),
            ImageCommand::Ls => cli::image_cmd::ls(&root),
            ImageCommand::Rm { reference } => cli::image_cmd::rm(&root, &reference),
        },
    }?;
    Ok(0)
}
