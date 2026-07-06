//! The commit graph: [`CommitsData`], [`Commit`], [`Head`] and read-only
//! reference/iterator types.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use orca_hash::{Hash, from_hex, hash_serde, to_hex};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::builder::CommitBuilder;

/// Where HEAD points: a branch (normal) or a raw commit (detached).
///
/// Serialized in commits.toml as a single string: `"branch:<name>"` or
/// `"commit:<hex>"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// HEAD follows a branch.
    Branch(String),
    /// Detached HEAD pinned to a commit. Committing is not allowed here.
    Detached(Hash),
}

impl fmt::Display for Head {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Head::Branch(name) => write!(f, "branch:{name}"),
            Head::Detached(hash) => write!(f, "commit:{}", to_hex(hash)),
        }
    }
}

impl FromStr for Head {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(name) = s.strip_prefix("branch:") {
            Ok(Head::Branch(name.to_string()))
        } else if let Some(hex) = s.strip_prefix("commit:") {
            let hash = from_hex(hex).map_err(|e| e.to_string())?;
            Ok(Head::Detached(hash))
        } else {
            Err(format!("invalid head value: {s}"))
        }
    }
}

impl Serialize for Head {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Head {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A named branch pointing at a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Branch {
    pub(crate) name: String,
    #[serde(with = "hash_serde")]
    pub(crate) commit: Hash,
}

/// A single commit: an immutable hash, mutable parent pointers, and an
/// optional layer reference.
///
/// Invariants:
/// - `hash` is computed once by [`CommitBuilder`] and never changes
///   (rebase only rewrites `parent`).
/// - `layer`, when `Some`, equals `hash` and names `layers/<hash>/`.
///   `None` means the commit carries no layer (the initial commit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    #[serde(with = "hash_serde")]
    pub(crate) hash: Hash,
    #[serde(with = "hash_serde::vec")]
    pub(crate) parent: Vec<Hash>,
    #[serde(
        with = "hash_serde::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) layer: Option<Hash>,
    pub(crate) message: String,
    pub(crate) timestamp: DateTime<Utc>,
}

impl Commit {
    /// The immutable commit identifier.
    pub fn hash(&self) -> &Hash {
        &self.hash
    }

    /// The commit message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Creation time of the commit (UTC).
    pub fn timestamp(&self) -> &DateTime<Utc> {
        &self.timestamp
    }

    /// The layer this commit references (`layers/<hash>/`), or `None` for
    /// layer-less commits (the initial commit).
    pub fn layer_hash(&self) -> Option<&Hash> {
        self.layer.as_ref()
    }

    /// Whether this is a root commit (no parents). Each environment has
    /// exactly one, which the `ROOT` alias resolves to.
    pub fn is_root(&self) -> bool {
        self.parent.is_empty()
    }

    /// Parent pointers (currently 0 or 1; `Vec` for future merge support).
    pub fn parents(&self) -> &[Hash] {
        &self.parent
    }
}

/// The whole commit graph of one environment: commits, branches, and HEAD.
///
/// This is a pure in-memory value; persistence lives in
/// [`CommitStore`](crate::CommitStore) and mutation in [`Vcs`](crate::Vcs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitsData {
    pub(crate) head: Head,
    pub(crate) branches: Vec<Branch>,
    pub(crate) commits: Vec<Commit>,
}

impl CommitsData {
    /// Default branch name used for the initial commit.
    pub const DEFAULT_BRANCH: &'static str = "main";

    /// Create a fresh graph containing the auto-generated initial commit
    /// (`parent=[]`, `layer=None`) with HEAD on `main`.
    pub fn new() -> Self {
        let root = CommitBuilder::new()
            .timestamp_now()
            .build()
            .expect("building the initial commit cannot fail");
        let branches = vec![Branch {
            name: Self::DEFAULT_BRANCH.to_string(),
            commit: root.hash,
        }];
        Self {
            head: Head::Branch(Self::DEFAULT_BRANCH.to_string()),
            branches,
            commits: vec![root],
        }
    }

    /// A reference to the current HEAD.
    pub fn head(&self) -> HeadRef<'_> {
        HeadRef { data: self }
    }

    /// Iterate over all branches.
    pub fn branches(&self) -> impl Iterator<Item = BranchRef<'_>> {
        self.branches
            .iter()
            .map(|b| BranchRef { data: self, branch: b })
    }

    /// Look up a branch by name.
    pub fn branch(&self, name: &str) -> Option<BranchRef<'_>> {
        self.branches
            .iter()
            .find(|b| b.name == name)
            .map(|b| BranchRef { data: self, branch: b })
    }

    /// Look up a commit by exact hash.
    pub fn find(&self, hash: &Hash) -> Option<&Commit> {
        self.commits.iter().find(|c| &c.hash == hash)
    }

    /// Look up commits whose hex hash starts with `prefix`.
    pub fn find_by_prefix(&self, prefix: &str) -> Vec<&Commit> {
        self.commits
            .iter()
            .filter(|c| to_hex(&c.hash).starts_with(prefix))
            .collect()
    }

    /// The unique root commit (`parent=[]`), target of the `ROOT` alias.
    ///
    /// Panics only if the graph is corrupt (no root commit), which cannot
    /// happen for graphs created via [`CommitsData::new`].
    pub fn root(&self) -> &Commit {
        self.commits
            .iter()
            .find(|c| c.is_root())
            .expect("commit graph has no root commit")
    }

    /// Iterate all commits reachable from HEAD in BFS order over all
    /// parents (first-parent plus future merge parents).
    pub fn commits(&self) -> CommitIter<'_> {
        let start = self.head_commit_hash();
        CommitIter::bfs(self, start)
    }

    /// Every commit stored in the graph, in insertion order (also the ones
    /// not reachable from HEAD).
    pub fn all_commits(&self) -> impl Iterator<Item = &Commit> {
        self.commits.iter()
    }

    /// The set of commits reachable from any branch head or from HEAD
    /// (including detached HEAD). Used as gc roots.
    pub fn reachable(&self) -> HashSet<Hash> {
        let mut roots: Vec<Hash> = self.branches.iter().map(|b| b.commit).collect();
        roots.push(self.head_commit_hash());
        let mut seen = HashSet::new();
        let mut queue: VecDeque<Hash> = roots.into_iter().collect();
        while let Some(hash) = queue.pop_front() {
            if !seen.insert(hash) {
                continue;
            }
            if let Some(commit) = self.find(&hash) {
                queue.extend(commit.parent.iter().copied());
            }
        }
        seen
    }

    /// The commit HEAD currently points at, or `None` if the graph is
    /// corrupt (branch pointing at a missing commit).
    pub fn current_commit(&self) -> Option<&Commit> {
        self.find(&self.head_commit_hash())
    }

    /// Resolve HEAD to a commit hash (branch tip or detached hash).
    pub(crate) fn head_commit_hash(&self) -> Hash {
        match &self.head {
            Head::Branch(name) => self
                .branches
                .iter()
                .find(|b| &b.name == name)
                .map(|b| b.commit)
                .unwrap_or([0; 32]),
            Head::Detached(hash) => *hash,
        }
    }
}

impl Default for CommitsData {
    fn default() -> Self {
        Self::new()
    }
}

/// Read-only reference to HEAD.
pub struct HeadRef<'a> {
    data: &'a CommitsData,
}

impl<'a> HeadRef<'a> {
    /// Walk history from HEAD following first parents (newest first).
    pub fn commits(&self) -> CommitIter<'a> {
        CommitIter::first_parent(self.data, self.data.head_commit_hash())
    }

    /// Whether HEAD is detached (pinned to a commit, not a branch).
    pub fn is_detached(&self) -> bool {
        matches!(self.data.head, Head::Detached(_))
    }

    /// The branch name HEAD follows, if not detached.
    pub fn branch_name(&self) -> Option<&'a str> {
        match &self.data.head {
            Head::Branch(name) => Some(name.as_str()),
            Head::Detached(_) => None,
        }
    }

    /// The commit hash HEAD resolves to.
    pub fn commit_hash(&self) -> Hash {
        self.data.head_commit_hash()
    }
}

/// Read-only reference to a branch.
pub struct BranchRef<'a> {
    data: &'a CommitsData,
    branch: &'a Branch,
}

impl<'a> BranchRef<'a> {
    /// The branch name.
    pub fn name(&self) -> &'a str {
        &self.branch.name
    }

    /// The commit hash the branch points at.
    pub fn commit_hash(&self) -> &'a Hash {
        &self.branch.commit
    }

    /// Walk history from the branch tip following first parents.
    pub fn commits(&self) -> CommitIter<'a> {
        CommitIter::first_parent(self.data, self.branch.commit)
    }
}

/// Iterator over commits, either along the first-parent chain or in BFS
/// order over all parents.
pub struct CommitIter<'a> {
    data: &'a CommitsData,
    mode: IterMode,
}

enum IterMode {
    FirstParent { next: Option<Hash> },
    Bfs { queue: VecDeque<Hash>, seen: HashSet<Hash> },
}

impl<'a> CommitIter<'a> {
    fn first_parent(data: &'a CommitsData, start: Hash) -> Self {
        Self {
            data,
            mode: IterMode::FirstParent { next: Some(start) },
        }
    }

    fn bfs(data: &'a CommitsData, start: Hash) -> Self {
        Self {
            data,
            mode: IterMode::Bfs {
                queue: VecDeque::from([start]),
                seen: HashSet::new(),
            },
        }
    }
}

impl<'a> Iterator for CommitIter<'a> {
    type Item = &'a Commit;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.mode {
            IterMode::FirstParent { next } => {
                let hash = next.take()?;
                let commit = self.data.find(&hash)?;
                *next = commit.parent.first().copied();
                Some(commit)
            }
            IterMode::Bfs { queue, seen } => loop {
                let hash = queue.pop_front()?;
                if !seen.insert(hash) {
                    continue;
                }
                let Some(commit) = self.data.find(&hash) else {
                    continue;
                };
                queue.extend(commit.parent.iter().copied());
                return Some(commit);
            },
        }
    }
}
