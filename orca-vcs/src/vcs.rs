use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug)]
pub struct VCS {
    commits_file_path: PathBuf,
    commits_data: CommitsData,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CommitsData {
    commits: Vec<Commit>,
    head: Head,
    branches: Vec<Branch>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Head {
    branch_name: String,
    commit_id: String,
    detached: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Branch {
    name: String,
    commit_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Commit {
    pub id: String,
    parent_id: Option<String>,
    pub date: String,
    pub message: Option<String>,
}

#[derive(Debug)]
pub struct CommitsIter<T> {
    commits: Vec<T>,
    head_id: Option<String>,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug)]
enum CommitQuery<T> {
    HEAD,
    Branch(T),
    CommitID(T),
    Other(T),
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Not initialized")]
    NotInitialized,

    #[error("Cannot operate to commits file")]
    FileOperationError(#[from] io::Error),

    #[error("Commits files is invalid format")]
    InvalidFormat,

    #[error("Cannot commit with detached HEAD")]
    DetachedHEAD,

    #[error("Specified branch name already exists")]
    BranchAllreadyExits,

    #[error("Specified commit was not found")]
    CommitNotFound,

    #[error("Specified commit id matches more than one commits")]
    AmbigousQuery,
}

const DEFAULT_BRANCH: &str = "main";
pub type Result<T> = std::result::Result<T, Error>;

impl VCS {
    pub fn new<P: Into<PathBuf>>(commits_file_path: P) -> Result<Self> {
        let commits_file_path = commits_file_path.into();
        if !commits_file_path.exists() {
            Err(Error::NotInitialized)?;
        }
        let mut commits_file = File::open(&commits_file_path)?;
        let mut buf = String::new();
        commits_file.read_to_string(&mut buf)?;
        let commits_data: CommitsData = toml::from_str(&buf).or(Err(Error::InvalidFormat))?;

        Ok(Self {
            commits_file_path,
            commits_data,
        })
    }

    pub fn init<P: Into<PathBuf>>(commits_file_path: P) -> Result<()> {
        let commits_file_path = commits_file_path.into();
        let commits_data = CommitsData::new();

        let commits_file_dir = commits_file_path.parent();
        if let Some(commits_file_dir) = commits_file_dir {
            if !commits_file_dir.exists() {
                fs::create_dir_all(commits_file_dir)?;
            }
        }
        write_commit_data_to_file(&commits_file_path, &commits_data)?;

        Ok(())
    }

    pub fn commit<S>(&mut self, message: Option<S>) -> Result<&Commit>
    where
        S: ToString,
    {
        let head_commit = self.commits_data.get_commit_by(CommitQuery::<&str>::HEAD);
        let parent_id = match head_commit {
            Ok(commit) => Some(commit.id.clone()),
            Err(Error::CommitNotFound) => None,
            Err(_) => {
                panic!("Multiple commits have the same commit id, or HEAD is invalid")
            }
        };
        let new_commit = Commit::new(parent_id, message);

        // TODO: DB
        // let mut current_tag = self.commits_data.get_current_tag().ok_or(VCSError::DetachedHEAD)?;
        // current_tag.commit_id = new_commit.id.clone();
        // self.commits_data.update_tag(current_tag);
        //
        self.commits_data
            .get_current_branch_mut()
            .ok_or(Error::DetachedHEAD)?
            .commit_id = new_commit.id.clone();
        self.commits_data.head.commit_id = new_commit.id.clone();
        self.commits_data.add_commit(new_commit);

        write_commit_data_to_file(&self.commits_file_path, &self.commits_data)?;

        Ok(self
            .commits_data
            .get_commit_by(CommitQuery::<&str>::HEAD)
            .unwrap())
    }

    pub fn get_current_commits(&self) -> Result<CommitsIter<&Commit>> {
        self.commits_data.get_commits_by(CommitQuery::<&str>::HEAD)
    }

    pub fn get_current_branch(&self) -> &str {
        self.commits_data
            .get_current_branch()
            .unwrap()
            .name
            .as_str()
    }

    pub fn get_all_branches(&self) -> Vec<&str> {
        self.commits_data
            .get_all_branches()
            .iter()
            .map(|tag| tag.name.as_str())
            .collect()
    }

    pub fn create_branch<S: ToString>(&mut self, name: S) -> anyhow::Result<()> {
        if self
            .commits_data
            .get_all_branches()
            .iter()
            .any(|tag| tag.name == name.to_string())
        {
            Err(Error::BranchAllreadyExits)?;
        }

        let latest_commit_id = self
            .commits_data
            .get_commit_by(CommitQuery::<&str>::HEAD)
            .map_or(String::from("none"), |commit| commit.id.clone());
        self.commits_data.add_branch(name, latest_commit_id);

        write_commit_data_to_file(&self.commits_file_path, &self.commits_data)?;
        Ok(())
    }

    pub fn delete_branch<S: ToString>(&mut self, _name: S) {
        unimplemented!();
    }

    pub fn checkout<S: ToString>(&mut self, query: S) -> Result<()> {
        let commit_query = create_commit_query_from(query, &self.commits_data);
        let commit = self.commits_data.get_commit_by(commit_query.clone())?;
        self.commits_data.head.commit_id = commit.id.clone();
        match commit_query {
            CommitQuery::Branch(tag) => {
                self.commits_data.head.branch_name = tag;
                self.commits_data.head.detached = false;
            }
            CommitQuery::CommitID(_) => self.commits_data.head.detached = true,
            _ => {}
        }
        write_commit_data_to_file(&self.commits_file_path, &self.commits_data)?;
        Ok(())
    }
}

fn write_commit_data_to_file<P: AsRef<Path>>(
    file_path: P,
    commits_data: &CommitsData,
) -> std::result::Result<(), io::Error> {
    let commits_toml = toml::to_string(commits_data).unwrap();
    let commits_file = File::create(file_path)?;
    let mut writer = BufWriter::new(commits_file);
    writer.write_all(commits_toml.as_bytes())?;
    Ok(())
}

fn create_commit_query_from<S: ToString>(
    query: S,
    commits_data: &CommitsData,
) -> CommitQuery<String> {
    let query = query.to_string();
    if query == "HEAD" {
        CommitQuery::HEAD
    } else if commits_data
        .get_all_branches()
        .iter()
        .any(|tag| tag.name.as_str() == query)
    {
        CommitQuery::Branch(query)
    } else {
        CommitQuery::CommitID(query)
    }
}

impl CommitsData {
    fn new() -> Self {
        let head = Head {
            branch_name: DEFAULT_BRANCH.to_string(),
            commit_id: "None".to_string(),
            detached: false,
        };
        let tags = vec![Branch {
            name: DEFAULT_BRANCH.to_string(),
            commit_id: "None".to_string(),
        }];
        Self {
            commits: vec![],
            head,
            branches: tags,
        }
    }

    fn add_commit(&mut self, commit: Commit) {
        self.commits.push(commit);
    }

    fn add_branch<S1, S2>(&mut self, name: S1, commit_id: S2)
    where
        S1: ToString,
        S2: ToString,
    {
        self.branches.push(Branch {
            name: name.to_string(),
            commit_id: commit_id.to_string(),
        })
    }

    fn get_current_branch(&self) -> Option<&Branch> {
        if self.head.detached {
            return None;
        }
        self.branches
            .iter()
            .find(|tag| tag.name.as_str() == self.head.branch_name.as_str())
    }

    fn get_current_branch_mut(&mut self) -> Option<&mut Branch> {
        if self.head.detached {
            return None;
        }
        self.branches
            .iter_mut()
            .find(|tag| tag.name.as_str() == self.head.branch_name.as_str())
    }

    fn get_all_branches(&self) -> &Vec<Branch> {
        &self.branches
    }

    fn get_commit_by<S: AsRef<str>>(&self, query: CommitQuery<S>) -> Result<&Commit> {
        let commit_id = get_commit_id_from_query(self, query).ok_or(Error::CommitNotFound)?;
        let commits: Vec<&Commit> = self
            .commits
            .iter()
            .filter(|commit| commit.id.starts_with(&commit_id))
            .collect();
        match commits.len() {
            1 => Ok(commits[0]),
            0 => Err(Error::CommitNotFound),
            _ => Err(Error::AmbigousQuery),
        }
    }

    fn get_commits_by<S: AsRef<str>>(&self, query: CommitQuery<S>) -> Result<CommitsIter<&Commit>> {
        let commit = self.get_commit_by(query)?;
        let head_id = commit.id.to_string();
        Ok(CommitsIter {
            commits: self.commits.iter().collect(),
            head_id: Some(head_id),
        })
    }
}

fn get_commit_id_from_query<S: AsRef<str>>(
    commits_data: &CommitsData,
    query: CommitQuery<S>,
) -> Option<String> {
    match query {
        CommitQuery::HEAD => Some(commits_data.head.commit_id.clone()),
        CommitQuery::Branch(tag_name) => commits_data
            .branches
            .iter()
            .find(|tag| tag.name.as_str() == tag_name.as_ref())
            .map(|branch| branch.commit_id.clone()),
        CommitQuery::CommitID(id) => Some(id.as_ref().to_string()),
        CommitQuery::Other(query) => {
            match get_commit_id_from_query(commits_data, CommitQuery::Branch(query.as_ref())) {
                Some(commit_id) => Some(commit_id),
                None => {
                    get_commit_id_from_query(commits_data, CommitQuery::CommitID(query.as_ref()))
                }
            }
        }
    }
}

impl Commit {
    fn new<S1, S2>(parent_id: Option<S1>, message: Option<S2>) -> Self
    where
        S1: ToString,
        S2: ToString,
    {
        let now = Local::now().to_string();
        let mut hasher = Sha1::new();
        hasher.update(&now);
        let id = hasher
            .finalize()
            .iter()
            .map(|data| format!("{:02x}", data))
            .collect::<String>();

        Self {
            id,
            date: now,
            message: message.map(|s| s.to_string()),
            parent_id: parent_id.map(|s| s.to_string()),
        }
    }
}

impl AsRef<Commit> for Commit {
    fn as_ref(&self) -> &Commit {
        self
    }
}

impl<T: AsRef<Commit>> Iterator for CommitsIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        if self.commits.is_empty() {
            return None;
        }
        let head_id = self.head_id.as_ref()?;
        let target_index = self
            .commits
            .iter()
            .position(|commit| commit.as_ref().id.as_str() == head_id)?;
        let target_commit = self.commits.swap_remove(target_index);
        self.head_id = target_commit.as_ref().parent_id.clone();
        Some(target_commit)
    }
}

#[allow(dead_code)]
impl<T> CommitQuery<T> {
    pub fn is_head(&self) -> bool {
        matches!(self, CommitQuery::HEAD)
    }

    pub fn is_tag(&self) -> bool {
        matches!(self, CommitQuery::Branch(_))
    }

    pub fn is_commit(&self) -> bool {
        matches!(self, CommitQuery::CommitID(_))
    }
}

impl<T: Clone> Clone for CommitQuery<T> {
    fn clone(&self) -> Self {
        match self {
            CommitQuery::HEAD => CommitQuery::HEAD,
            CommitQuery::Branch(t) => CommitQuery::Branch(t.clone()),
            CommitQuery::CommitID(c) => CommitQuery::CommitID(c.clone()),
            CommitQuery::Other(q) => CommitQuery::Other(q.clone()),
        }
    }
}
impl<T: Copy> Copy for CommitQuery<T> {}
