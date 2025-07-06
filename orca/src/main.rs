mod args;

use anyhow::{bail, Result};
use args::{Action, Args, RunArgs};
use clap::Parser;
use nix::unistd::{getegid, geteuid};
use orca_container::container::Container;
use orca_image::HostImage;
use orca_vcs::{Commit, CommitsIter, Error, VCS};
use std::env;
use std::fs::{create_dir_all, rename};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const COMMITS_FILE_NAME: &str = "commits.toml";
const MOUNTPOINT_DIR_NAME: &str = "rootfs";
const UPPER_DIR_NAME: &str = "upper";
const WORK_DIR_NAME: &str = "work";
const LOWER_DIR_NAME: &str = "layers";
const TMP_DIR_NAME: &str = "tmp";

fn main() -> Result<()> {
    let args = Args::parse();
    run(args)?;
    Ok(())
}

fn run(args: Args) -> Result<()> {
    let rootdir = args.root;
    let env_root = PathBuf::from(rootdir).join(args.name);
    let commits_file = env_root.join(COMMITS_FILE_NAME);
    let mount_point = env_root.join(MOUNTPOINT_DIR_NAME);
    let upperdir = env_root.join(UPPER_DIR_NAME);
    let workdir = env_root.join(WORK_DIR_NAME);
    let lower_root = env_root.join(LOWER_DIR_NAME);
    let tmpdir = env_root.join(TMP_DIR_NAME);

    if matches!(args.action, Action::Init(_)) {
        create_dir_all(&mount_point)?;
        create_dir_all(&upperdir)?;
        create_dir_all(&workdir)?;
        create_dir_all(&lower_root)?;
        create_dir_all(&tmpdir)?;
        VCS::init(&commits_file)?;
        return Ok(());
    }

    let mut vcs = match VCS::new(&commits_file) {
        Ok(vcs) => vcs,
        Err(Error::NotInitialized) => bail!("You have to initialize with 'init'"),
        Err(e) => Err(e)?,
    };

    match args.action {
        Action::Init(_) => {
            // do nothing
            Ok(())
        }

        Action::Run(args) => {
            if !is_root() {
                bail!(
                    "'Run' needs root priviledge!
                    Execute with 'sudo' or setuid to binary!"
                );
            }

            let commits: Vec<&Commit> = match vcs.get_current_commits() {
                Ok(commits) => commits.collect(),
                Err(Error::CommitNotFound) => vec![],
                Err(e) => Err(e)?,
            };
            let lowerdirs = create_lowerdirs_from_commits(commits, lower_root);
            let argv = run_args_to_vec(args, env::var("SHELL").unwrap());
            let image = HostImage::new(mount_point, upperdir, lowerdirs, workdir, &tmpdir);
            let working_container = Container::new(image, argv, tmpdir)?;
            working_container.wait()?;

            Ok(())
        }
        Action::Log(_) => {
            let commits = match vcs.get_current_commits() {
                Ok(commits) => commits,
                Err(Error::CommitNotFound) => bail!("Current branch does not have any commits"),
                Err(e) => Err(e)?,
            };
            print_commits_info(commits);
            Ok(())
        }
        Action::Commit(args) => {
            let commit = vcs.commit(args.message)?;
            rename(&upperdir, lower_root.join(commit.id.as_str()))?;
            create_dir_all(&upperdir)?;
            println!("{}", commit.id.as_str());
            Ok(())
        }
        Action::Branch(args) => {
            if args.all {
                let branches = vcs.get_all_branches();
                print_all_branches(branches);
                Ok(())
            } else if args.delete {
                vcs.delete_branch(args.branch_name.unwrap());
                Ok(())
            } else if let Some(branch_name) = args.branch_name {
                vcs.create_branch(branch_name)
            } else {
                let branches = vcs.get_current_branch();
                println!("{branches}");
                Ok(())
            }
        }
        Action::Checkout(args) => {
            if upperdir.read_dir()?.next().is_some() {
                bail!("You have commit first");
            }
            vcs.checkout(args.query)?;
            Ok(())
        }
        Action::Merge(_) => {
            unimplemented!();
        }
        Action::Reset(_) => {
            unimplemented!();
        }
        Action::Diff => {
            print_dir_content_recursively(upperdir);
            Ok(())
        }
        Action::Clean => {
            std::fs::remove_dir_all(&upperdir)?;
            std::fs::create_dir_all(&upperdir)?;
            Ok(())
        }
    }
}

fn is_root() -> bool {
    getegid().as_raw() == 0 && geteuid().as_raw() == 0
}

fn create_lowerdirs_from_commits<P: AsRef<Path>>(
    commits: Vec<&Commit>,
    rootdir: P,
) -> Vec<PathBuf> {
    commits
        .into_iter()
        .map(|commit| rootdir.as_ref().join(commit.id.as_str()))
        .collect()
}

fn run_args_to_vec<S: ToString>(args: RunArgs, default_cmd: S) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    match args.command {
        Some(cmd) => {
            v.push(cmd);
            if let Some(mut cmd_args) = args.args {
                v.append(&mut cmd_args);
            }
        }
        None => v.push(default_cmd.to_string()),
    }
    v
}

fn print_commits_info(commits: CommitsIter<&Commit>) {
    for commit in commits {
        println!("commit: {}", commit.id);
        println!("  date: {}", commit.date);
        if let Some(message) = commit.message.as_ref() {
            println!("  message: {}", message);
        }
        println!();
    }
}

fn print_all_branches(branches: Vec<&str>) {
    for branch in branches {
        println!("{branch}");
    }
}

fn print_dir_content_recursively<P: AsRef<Path>>(root_path: P) {
    let mut walkdir = WalkDir::new(root_path.as_ref()).into_iter();
    let _ = walkdir.next();
    let root = PathBuf::from("/");
    for entry in walkdir {
        let entry = entry.unwrap();
        let metadata = entry.metadata().unwrap();
        let path = root.join(entry.path().strip_prefix(root_path.as_ref()).unwrap());
        if !metadata.is_dir() {
            if entry.file_type().is_char_device() && metadata.rdev() == 0 {
                println!("- {}", path.display());
            } else {
                println!("+ {}", path.display());
            }
        }
    }
}
