//! [`Vcs`]: the version-control operations over [`CommitsData`].

use std::collections::HashSet;

use orca_hash::{Hash, to_hex};

use crate::data::{CommitsData, Head};
use crate::{Commit, VcsError};

/// Stateless operations over a [`CommitsData`] graph.
///
/// All methods mutate the in-memory graph only; persisting the result
/// (and any layer-store side effects) is the caller's responsibility.
/// Preconditions that involve the environment (no running container,
/// empty `diff/`) are checked by the caller (`Workspace` in the `orca`
/// crate), not here.
pub struct Vcs;

impl Vcs {
    /// Append `commit` to the graph and advance the current branch to it.
    ///
    /// Returns [`VcsError::DetachedHead`] if HEAD is not on a branch
    /// (committing in detached HEAD is not allowed).
    pub fn commit(data: &mut CommitsData, commit: Commit) -> Result<(), VcsError> {
        let Head::Branch(name) = data.head.clone() else {
            return Err(VcsError::DetachedHead);
        };
        let branch = data
            .branches
            .iter_mut()
            .find(|b| b.name == name)
            .ok_or(VcsError::BranchNotFound(name))?;
        branch.commit = commit.hash;
        data.commits.push(commit);
        Ok(())
    }

    /// Move HEAD to `target`.
    ///
    /// `Head::Branch` must name an existing branch; `Head::Detached` must
    /// reference an existing commit. The caller (`Workspace::checkout`)
    /// resolves user input (branch / hash / `ROOT`) into a [`Head`].
    pub fn checkout(data: &mut CommitsData, target: &Head) -> Result<(), VcsError> {
        match target {
            Head::Branch(name) => {
                if data.branch(name).is_none() {
                    return Err(VcsError::BranchNotFound(name.clone()));
                }
            }
            Head::Detached(hash) => {
                if data.find(hash).is_none() {
                    return Err(VcsError::CommitNotFound(to_hex(hash)));
                }
            }
        }
        data.head = target.clone();
        Ok(())
    }

    /// Hard reset: move the branch HEAD points at to `target`.
    ///
    /// Returns [`VcsError::DetachedHead`] when HEAD is detached (reset
    /// moves a branch pointer; use checkout to move a detached HEAD).
    pub fn reset(data: &mut CommitsData, target: &Commit) -> Result<(), VcsError> {
        let target_hash = *target.hash();
        if data.find(&target_hash).is_none() {
            return Err(VcsError::CommitNotFound(to_hex(&target_hash)));
        }
        let Head::Branch(name) = data.head.clone() else {
            return Err(VcsError::DetachedHead);
        };
        let branch = data
            .branches
            .iter_mut()
            .find(|b| b.name == name)
            .ok_or(VcsError::BranchNotFound(name))?;
        branch.commit = target_hash;
        Ok(())
    }

    /// Create a branch named `name` at the current HEAD commit.
    pub fn branch_create(data: &mut CommitsData, name: &str) -> Result<(), VcsError> {
        if data.branch(name).is_some() {
            return Err(VcsError::BranchExists(name.to_string()));
        }
        let commit = data.head_commit_hash();
        data.branches.push(crate::data::Branch {
            name: name.to_string(),
            commit,
        });
        Ok(())
    }

    /// Delete the branch named `name`.
    ///
    /// The branch HEAD currently follows cannot be deleted.
    pub fn branch_delete(data: &mut CommitsData, name: &str) -> Result<(), VcsError> {
        if data.branch(name).is_none() {
            return Err(VcsError::BranchNotFound(name.to_string()));
        }
        if data.head == Head::Branch(name.to_string()) {
            return Err(VcsError::BranchInUse(name.to_string()));
        }
        data.branches.retain(|b| b.name != name);
        Ok(())
    }

    /// Rebase branch `target` onto branch `newbase`.
    ///
    /// Pure pointer surgery: the oldest commit of `target` past the common
    /// ancestor gets its first parent redirected to `newbase`'s tip, and
    /// `newbase` is advanced to `target`'s tip. Hashes are never
    /// recomputed and no layer is renamed (see DESIGN §6). Shared commits
    /// must not be rebased — the parent rewrite would leak into other
    /// branches referencing them.
    pub fn rebase(data: &mut CommitsData, newbase: &str, target: &str) -> Result<(), VcsError> {
        let newbase_tip = *data
            .branch(newbase)
            .ok_or_else(|| VcsError::BranchNotFound(newbase.to_string()))?
            .commit_hash();
        let target_tip = *data
            .branch(target)
            .ok_or_else(|| VcsError::BranchNotFound(target.to_string()))?
            .commit_hash();

        let lca = Self::find_lca(data, &newbase_tip, &target_tip)?;
        if target_tip == lca {
            // target has nothing beyond the common ancestor.
            return Err(VcsError::NothingToRebase {
                target: target.to_string(),
                newbase: newbase.to_string(),
            });
        }

        if newbase_tip != lca {
            // Find the oldest commit on target's first-parent chain past
            // the LCA and re-parent it onto newbase's tip.
            let mut cursor = target_tip;
            let first_after_lca = loop {
                let commit = data
                    .find(&cursor)
                    .ok_or_else(|| VcsError::CommitNotFound(to_hex(&cursor)))?;
                match commit.parent.first() {
                    Some(parent) if *parent == lca => break cursor,
                    Some(parent) => cursor = *parent,
                    None => {
                        return Err(VcsError::NoCommonAncestor(
                            newbase.to_string(),
                            target.to_string(),
                        ));
                    }
                }
            };
            let commit = data
                .commits
                .iter_mut()
                .find(|c| c.hash == first_after_lca)
                .expect("commit existence checked above");
            commit.parent[0] = newbase_tip;
        }
        // Advance newbase to target's tip (fast-forward when newbase == lca).
        let branch = data
            .branches
            .iter_mut()
            .find(|b| b.name == newbase)
            .expect("branch existence checked above");
        branch.commit = target_tip;
        Ok(())
    }

    /// Reserved for the future `orca merge`; always returns
    /// [`VcsError::MergeUnimplemented`].
    pub fn merge(_data: &mut CommitsData, _target: &str) -> Result<(), VcsError> {
        Err(VcsError::MergeUnimplemented)
    }

    /// Lowest common ancestor of two commits along first-parent chains.
    pub fn find_lca(data: &CommitsData, a: &Hash, b: &Hash) -> Result<Hash, VcsError> {
        let mut ancestors = HashSet::new();
        let mut cursor = Some(*a);
        while let Some(hash) = cursor {
            ancestors.insert(hash);
            cursor = data.find(&hash).and_then(|c| c.parent.first().copied());
        }
        let mut cursor = Some(*b);
        while let Some(hash) = cursor {
            if ancestors.contains(&hash) {
                return Ok(hash);
            }
            cursor = data.find(&hash).and_then(|c| c.parent.first().copied());
        }
        Err(VcsError::NoCommonAncestor(to_hex(a), to_hex(b)))
    }

    /// Mark-and-sweep garbage collection (pure, no I/O).
    ///
    /// Roots are every branch tip plus the current HEAD (including a
    /// detached HEAD — otherwise the checked-out commit would be swept).
    /// Returns the cleaned graph and the layer hashes of removed commits;
    /// deleting those layer directories is the caller's job
    /// (`Workspace::gc` → `LayerStore::delete`).
    pub fn gc(data: CommitsData) -> (CommitsData, Vec<Hash>) {
        let reachable = data.reachable();
        let mut kept = Vec::new();
        let mut dead_layers = Vec::new();
        for commit in data.commits {
            if reachable.contains(&commit.hash) {
                kept.push(commit);
            } else if let Some(layer) = commit.layer {
                dead_layers.push(layer);
            }
        }
        (
            CommitsData {
                head: data.head,
                branches: data.branches,
                commits: kept,
            },
            dead_layers,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommitBuilder;

    /// Append a commit with a synthetic layer hash on the current branch.
    fn add_commit(data: &mut CommitsData, msg: &str) -> Hash {
        struct Marker(&'static str, u64);
        impl orca_hash::Hashable for Marker {
            fn hash(&self, h: &mut dyn orca_hash::Hasher) {
                h.update_framed(self.0.as_bytes());
                h.update(&self.1.to_le_bytes());
            }
        }
        let parent = data.current_commit().unwrap().hash;
        let marker = Marker("layer", data.all_commits().count() as u64);
        let commit = CommitBuilder::new()
            .parent(parent)
            .message(msg)
            .data(&marker)
            .timestamp_now()
            .build()
            .unwrap();
        let hash = commit.hash;
        Vcs::commit(data, commit).unwrap();
        hash
    }

    #[test]
    fn commit_advances_branch() {
        let mut data = CommitsData::new();
        let hash = add_commit(&mut data, "first");
        assert_eq!(data.head().commit_hash(), hash);
        assert_eq!(data.head().commits().count(), 2);
    }

    #[test]
    fn commit_rejected_in_detached_head() {
        let mut data = CommitsData::new();
        let root = *data.root().hash();
        Vcs::checkout(&mut data, &Head::Detached(root)).unwrap();
        let commit = CommitBuilder::new().parent(root).timestamp_now().build().unwrap();
        assert!(matches!(
            Vcs::commit(&mut data, commit),
            Err(VcsError::DetachedHead)
        ));
    }

    #[test]
    fn reset_moves_branch_and_rejects_detached() {
        let mut data = CommitsData::new();
        let first = add_commit(&mut data, "first");
        add_commit(&mut data, "second");
        let target = data.find(&first).unwrap().clone();
        Vcs::reset(&mut data, &target).unwrap();
        assert_eq!(data.head().commit_hash(), first);

        Vcs::checkout(&mut data, &Head::Detached(first)).unwrap();
        assert!(matches!(
            Vcs::reset(&mut data, &target),
            Err(VcsError::DetachedHead)
        ));
    }

    #[test]
    fn branch_lifecycle() {
        let mut data = CommitsData::new();
        Vcs::branch_create(&mut data, "dev").unwrap();
        assert!(matches!(
            Vcs::branch_create(&mut data, "dev"),
            Err(VcsError::BranchExists(_))
        ));
        assert!(matches!(
            Vcs::branch_delete(&mut data, "main"),
            Err(VcsError::BranchInUse(_))
        ));
        Vcs::branch_delete(&mut data, "dev").unwrap();
        assert!(matches!(
            Vcs::branch_delete(&mut data, "dev"),
            Err(VcsError::BranchNotFound(_))
        ));
    }

    #[test]
    fn rebase_reparents_and_advances_newbase() {
        // main: root -> B -> C ; dev: root -> B -> D -> E
        let mut data = CommitsData::new();
        let _b = add_commit(&mut data, "B");
        Vcs::branch_create(&mut data, "dev").unwrap();
        let c = add_commit(&mut data, "C");
        Vcs::checkout(&mut data, &Head::Branch("dev".into())).unwrap();
        let d = add_commit(&mut data, "D");
        let e = add_commit(&mut data, "E");

        // rebase main dev -> main: root B C D E
        Vcs::rebase(&mut data, "main", "dev").unwrap();
        assert_eq!(*data.branch("main").unwrap().commit_hash(), e);
        assert_eq!(data.find(&d).unwrap().parents()[0], c);
        let chain: Vec<String> = data
            .branch("main")
            .unwrap()
            .commits()
            .map(|c| c.message().to_string())
            .collect();
        assert_eq!(chain, ["E", "D", "C", "B", ""]);
    }

    #[test]
    fn rebase_fast_forward_when_newbase_is_ancestor() {
        // main: root -> B ; dev: root -> B -> D
        let mut data = CommitsData::new();
        let b = add_commit(&mut data, "B");
        Vcs::branch_create(&mut data, "dev").unwrap();
        Vcs::checkout(&mut data, &Head::Branch("dev".into())).unwrap();
        let d = add_commit(&mut data, "D");
        Vcs::rebase(&mut data, "main", "dev").unwrap();
        assert_eq!(*data.branch("main").unwrap().commit_hash(), d);
        // D's parent is untouched (no re-parent needed).
        assert_eq!(data.find(&d).unwrap().parents()[0], b);
    }

    #[test]
    fn rebase_nothing_to_do() {
        let mut data = CommitsData::new();
        add_commit(&mut data, "B");
        Vcs::branch_create(&mut data, "dev").unwrap();
        // dev == main tip; rebasing dev onto main has nothing to move.
        assert!(matches!(
            Vcs::rebase(&mut data, "main", "dev"),
            Err(VcsError::NothingToRebase { .. })
        ));
    }

    #[test]
    fn gc_sweeps_unreachable_and_keeps_detached_head() {
        let mut data = CommitsData::new();
        let first = add_commit(&mut data, "first");
        let second = add_commit(&mut data, "second");
        // Move main back to first; second becomes unreachable.
        let target = data.find(&first).unwrap().clone();
        Vcs::reset(&mut data, &target).unwrap();
        let (cleaned, dead) = Vcs::gc(data);
        assert!(cleaned.find(&second).is_none());
        assert_eq!(dead, vec![second]);

        // Detached HEAD keeps its commit alive.
        let mut data = CommitsData::new();
        let first = add_commit(&mut data, "first");
        Vcs::checkout(&mut data, &Head::Detached(first)).unwrap();
        let root = *data.root().hash();
        let target = data.find(&root).unwrap().clone();
        // Move main back to root via a temporary checkout.
        Vcs::checkout(&mut data, &Head::Branch("main".into())).unwrap();
        Vcs::reset(&mut data, &target).unwrap();
        Vcs::checkout(&mut data, &Head::Detached(first)).unwrap();
        let (cleaned, dead) = Vcs::gc(data);
        assert!(cleaned.find(&first).is_some());
        assert!(dead.is_empty());
    }

    #[test]
    fn head_serializes_as_single_string() {
        let data = CommitsData::new();
        let toml = toml::to_string(&data).unwrap();
        assert!(toml.contains("head = \"branch:main\""));
        let parsed: CommitsData = toml::from_str(&toml).unwrap();
        assert_eq!(parsed.head().branch_name(), Some("main"));
    }
}
