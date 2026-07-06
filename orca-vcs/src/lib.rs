//! Git-like version control for orca environments.
//!
//! The commit graph is a DAG stored in `envs/<uuid>/commits.toml`
//! ([`CommitStore`]). [`CommitsData`] holds the graph (commits, branches,
//! HEAD), [`CommitBuilder`] computes the immutable commit hash, and [`Vcs`]
//! implements the operations (commit / checkout / reset / branch / rebase /
//! gc).
//!
//! This crate knows nothing about the on-disk layout of environments or
//! layers; callers (the `orca` crate) resolve paths and drive layer
//! promotion/deletion.

#![warn(missing_docs)]

mod builder;
mod data;
mod store;
mod vcs;

pub use builder::CommitBuilder;
pub use data::{BranchRef, Commit, CommitIter, CommitsData, Head, HeadRef};
pub use store::CommitStore;
pub use vcs::Vcs;

/// Errors returned by the version-control operations in this crate.
#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    /// A branch with the given name already exists.
    #[error("branch already exists: {0}")]
    BranchExists(String),
    /// No branch with the given name exists.
    #[error("branch not found: {0}")]
    BranchNotFound(String),
    /// No commit with the given hash exists in the graph.
    #[error("commit not found: {0}")]
    CommitNotFound(String),
    /// The operation requires HEAD to be on a branch (e.g. commit / reset).
    #[error("HEAD is detached; this operation requires HEAD to be on a branch")]
    DetachedHead,
    /// Attempted to delete the branch HEAD currently points to.
    #[error("cannot delete the current branch: {0}")]
    BranchInUse(String),
    /// The two rebase refs have no common ancestor (corrupt graph).
    #[error("no common ancestor between {0} and {1}")]
    NoCommonAncestor(String, String),
    /// Rebase target is already an ancestor of newbase (nothing to move).
    #[error("nothing to rebase: {target} is already contained in {newbase}")]
    NothingToRebase {
        /// The branch that was to be replayed.
        target: String,
        /// The branch it was to be replayed onto.
        newbase: String,
    },
    /// `orca merge` is reserved but not implemented.
    #[error("merge is not implemented yet; use rebase instead")]
    MergeUnimplemented,
    /// commits.toml could not be read or written.
    #[error("failed to access commit store: {0}")]
    Io(#[from] std::io::Error),
    /// commits.toml could not be parsed.
    #[error("failed to parse commits.toml: {0}")]
    Parse(#[from] toml::de::Error),
    /// commits.toml could not be serialized.
    #[error("failed to serialize commits.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
}
