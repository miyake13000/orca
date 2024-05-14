use clap::{Args as ArgsDerive, Parser, Subcommand};
use std::{
    env,
    path::{Path, PathBuf},
};

fn default_rootdir<P: AsRef<Path>>(dir_name: P) -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap()).join(dir_name)
}

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
    /// List initialized environment
    #[arg(short, long, default_value_t = false)]
    pub list: bool,

    /// Use specified named environment created by 'init'
    #[arg(short, long, default_value = "_default")]
    pub name: String,

    /// Root directory to save data [default: $HOME/.orca]
    #[arg(short, long, hide_default_value = true, default_value = default_rootdir(".orca").into_os_string()) ]
    pub root: String,

    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Initialize version controlled host environment
    /// (If --image is set, use container insted of host)
    Init(InitArgs),

    /// Run command in version controlled environment
    Run(RunArgs),

    /// Show commit logs
    Log(LogArgs),

    /// Record changes
    Commit(CommitArgs),

    /// List, create, or delete branches
    Branch(BranchArgs),

    /// Join two branches together
    Merge(TargetArgs),

    /// Show changes between commits
    Diff,

    /// Delete uncommited chenges
    Clean,

    /// Reset current branch to specified commit
    Reset(TargetArgs),

    /// Switch branches
    Checkout(TargetArgs),
}

#[derive(Debug, ArgsDerive)]
pub struct InitArgs {
    /// Use specified container image instead of Host image
    #[arg(short, long)]
    pub image: Option<String>,

    /// Tag name of specified container image
    #[arg(short, long, default_value = "latest")]
    pub tag: String,

    /// Assign a name to created environment
    #[arg(short, long, default_value = "_default")]
    pub name: String,
}

#[derive(Debug, ArgsDerive)]
pub struct RunArgs {
    /// Separate netns (If this option is set, you cannot use network without loopback)
    #[arg(long, default_value_t = false)]
    pub netns: bool,

    /// Command to execute
    pub command: Option<String>,

    /// Arguments of command
    #[arg(allow_hyphen_values = true)]
    pub args: Option<Vec<String>>,
}

#[derive(Debug, ArgsDerive)]
pub struct LogArgs {
    /// Show all branch
    #[arg(short, long)]
    pub all: bool,
}

#[derive(Debug, ArgsDerive)]
pub struct CommitArgs {
    /// Commit message
    #[arg(short, long)]
    pub message: Option<String>,
}

#[derive(Debug, ArgsDerive)]
pub struct BranchArgs {
    /// Branch name you want to create or delete
    pub branch_name: Option<String>,

    /// Delete specified branch
    #[arg(short, long, requires = "branch_name")]
    pub delete: bool,

    /// Show all branches
    #[arg(short, long, exclusive = true)]
    pub all: bool,
}

#[derive(Debug, ArgsDerive)]
pub struct TargetArgs {
    /// Commit ID or branch
    pub query: String,
}
