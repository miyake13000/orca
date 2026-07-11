//! [`Workspace`]: the facade for every operation touching history or
//! layers.
//!
//! A `Workspace` binds a resolved [`Env`] to its commit store, layer store
//! and loaded commit graph, and centralizes the preconditions (DESIGN §6):
//!
//! - operations that rename/destroy `diff/` or layers refuse to run while
//!   a container is live (`LockFile::check`; stale locks are cleaned);
//! - operations that change the lower stack (checkout / reset / rebase)
//!   additionally require an empty `diff/`.
//!
//! `orca run` is *not* here: the CLI drives `orca-container` directly,
//! sharing only the lock protocol (it acquires; we check).

use orca_hash::{Hash, to_hex};
use orca_image::external_image::{
    BlobStoreError, ImageIndex, IndexError, LayerBlobStore,
};
use orca_image::{
    Base, BaseImageRef, Blacklist, Change, DiffError, Image, ImageConfig, Layer, LayerStore,
    LayerStoreError, Upper, changes,
};
use orca_vcs::{Commit, CommitBuilder, CommitStore, CommitsData, Head, Vcs, VcsError};

use crate::apply::{self, ApplyError};
use crate::env::Env;
use crate::lock::{LockError, LockFile};
use crate::COMMIT_FILE_NAME;

/// Errors from workspace operations.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// A live container holds the environment.
    #[error("a container is running in this environment (pid {0}); stop it first")]
    Running(i32),
    /// The operation requires a clean upper (`diff/` empty).
    #[error("uncommitted changes exist; commit them or run `orca clean` first")]
    NotClean,
    /// The target string matched neither a branch, a commit, nor ROOT.
    #[error("unknown branch or commit: {0}")]
    UnknownTarget(String),
    /// A commit hash prefix matched more than one commit.
    #[error("ambiguous commit prefix: {0}")]
    AmbiguousTarget(String),
    /// apply is only supported for host-based environments.
    #[error("apply is only available for host-based environments")]
    HostOnly,
    /// An unfinished apply journal exists; resolve it first.
    #[error("an unfinished apply journal exists; run `orca apply` to resolve it")]
    ApplyPending,
    /// Version-control error.
    #[error(transparent)]
    Vcs(#[from] VcsError),
    /// Layer store error.
    #[error(transparent)]
    Layer(#[from] LayerStoreError),
    /// Diff computation error.
    #[error(transparent)]
    Diff(#[from] DiffError),
    /// Image index error.
    #[error(transparent)]
    Index(#[from] IndexError),
    /// Blob store error.
    #[error(transparent)]
    Blob(#[from] BlobStoreError),
    /// Lock error.
    #[error(transparent)]
    Lock(#[from] LockError),
    /// Apply error.
    #[error(transparent)]
    Apply(#[from] ApplyError),
    /// Miscellaneous I/O.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Facade over one environment's history and layers.
pub struct Workspace<'a> {
    env: &'a Env,
    commit_store: CommitStore,
    layer_store: LayerStore,
    commits: CommitsData,
    /// Parsed from the env's effective settings; applied to diff / apply
    /// (never to commit — DESIGN §6).
    blacklist: Blacklist,
}

impl<'a> Workspace<'a> {
    /// Open the workspace: construct the stores, load `commits.toml`,
    /// and parse the env's diff blacklist.
    pub fn open(env: &'a Env) -> Result<Self, WorkspaceError> {
        let commits_file = env.env_path().join(COMMIT_FILE_NAME);
        let commit_store = CommitStore::new(&commits_file);
        let layer_store = LayerStore::new(&env.layers_path());
        let commits = commit_store.load()?;
        let blacklist = Blacklist::parse(&env.settings().blacklist);
        Ok(Self {
            env,
            commit_store,
            layer_store,
            commits,
            blacklist,
        })
    }

    /// The blacklist to hand to `changes()` (None when unconfigured).
    fn blacklist(&self) -> Option<&Blacklist> {
        (!self.blacklist.is_empty()).then_some(&self.blacklist)
    }

    /// Read-only access to the commit graph (for `log` / `branch -a`
    /// rendering in the CLI).
    pub fn data(&self) -> &CommitsData {
        &self.commits
    }

    /// The environment this workspace operates on.
    pub fn env(&self) -> &Env {
        self.env
    }

    // ----- preconditions ---------------------------------------------

    /// Refuse to proceed while a live container holds the environment
    /// (stale locks are cleaned by `check`).
    fn ensure_not_running(&self) -> Result<(), WorkspaceError> {
        match LockFile::check(&self.env.lock_path())? {
            Some(pid) => Err(WorkspaceError::Running(pid)),
            None => Ok(()),
        }
    }

    /// Refuse to change the lower stack while `diff/` has content.
    fn ensure_clean(&self) -> Result<(), WorkspaceError> {
        if self.upper().is_empty()? {
            Ok(())
        } else {
            Err(WorkspaceError::NotClean)
        }
    }

    // ----- resolution helpers ----------------------------------------

    /// Resolve a user string to a commit hash: `ROOT`, a full hex hash,
    /// or a unique hex prefix (min 4 chars). Branch names are *not*
    /// resolved here — callers that accept branches check those first.
    fn resolve_commit(&self, target: &str) -> Result<Hash, WorkspaceError> {
        if target == "ROOT" {
            return Ok(*self.commits.root().hash());
        }
        if let Ok(hash) = orca_hash::from_hex(target)
            && self.commits.find(&hash).is_some()
        {
            return Ok(hash);
        }
        if target.len() >= 4 && target.chars().all(|c| c.is_ascii_hexdigit()) {
            let matches = self.commits.find_by_prefix(target);
            return match matches.len() {
                0 => Err(WorkspaceError::UnknownTarget(target.to_string())),
                1 => Ok(*matches[0].hash()),
                _ => Err(WorkspaceError::AmbiguousTarget(target.to_string())),
            };
        }
        Err(WorkspaceError::UnknownTarget(target.to_string()))
    }

    /// The layer stack (newest first) of the history ending at `commit`,
    /// following first parents down to the root.
    fn stack_of(&self, commit: Hash) -> Result<Vec<Layer>, WorkspaceError> {
        let mut hashes = Vec::new();
        let mut cursor = Some(commit);
        while let Some(hash) = cursor {
            let c = self
                .commits
                .find(&hash)
                .ok_or_else(|| WorkspaceError::UnknownTarget(to_hex(&hash)))?;
            if let Some(layer) = c.layer_hash() {
                hashes.push(*layer);
            }
            cursor = c.parents().first().copied();
        }
        Ok(self.layer_store.resolve(&hashes)?)
    }

    /// The upper as a value type.
    fn upper(&self) -> Upper {
        Upper::new(self.env.upper_path())
    }

    /// Assemble the full [`Image`] for the current HEAD (used by `orca
    /// run` via the CLI, and by diff).
    pub fn image(&self) -> Result<Image, WorkspaceError> {
        let head_hash = self.commits.head().commit_hash();
        let lower = self.stack_of(head_hash)?;
        let (base, config) = match &self.env.base_ref {
            // Host bases declare no static config; the runtime spec is
            // resolved by ExecSpec from the invocation context.
            BaseImageRef::Host => (Base::Host, ImageConfig::default()),
            BaseImageRef::External { image_digest } => {
                let index = ImageIndex::load(&self.env.images_path())?;
                let manifest = index.find_by_digest(image_digest)?;
                let blobs = LayerBlobStore::new(&self.env.blob_store_path());
                let layers = blobs.resolve(&manifest.layers_top_first())?;
                (Base::Guest(layers), manifest.config())
            }
        };
        Ok(Image {
            upper: self.upper(),
            lower,
            base,
            config,
        })
    }

    // ----- operations -------------------------------------------------

    /// Commit the upper: compute the immutable hash, promote `diff/` to
    /// `layers/<hash>/`, append to the graph, save.
    ///
    /// Preconditions: no running container; HEAD on a branch (detached
    /// commit is rejected by [`Vcs::commit`]).
    pub fn commit(&mut self, message: &str) -> Result<Hash, WorkspaceError> {
        self.ensure_not_running()?;
        let parent = self.commits.head().commit_hash();
        let upper = self.upper();
        let commit = CommitBuilder::new()
            .parent(parent)
            .message(message)
            .data(&upper)
            .timestamp_now()
            .build()?;
        let hash = *commit.hash();
        // Order matters for crash safety: record the commit only after
        // the layer exists. A crash between the two leaves an orphan
        // layer directory, which gc can reclaim.
        self.layer_store.promote_upper(&upper, &hash)?;
        Vcs::commit(&mut self.commits, commit)?;
        self.commit_store.save(&self.commits)?;
        Ok(hash)
    }

    /// Checkout a branch (HEAD follows it) or a commit / `ROOT` (detached
    /// HEAD). Preconditions: no running container, clean upper.
    pub fn checkout(&mut self, target: &str) -> Result<Head, WorkspaceError> {
        self.ensure_not_running()?;
        self.ensure_clean()?;
        let head = if self.commits.branch(target).is_some() {
            Head::Branch(target.to_string())
        } else {
            Head::Detached(self.resolve_commit(target)?)
        };
        Vcs::checkout(&mut self.commits, &head)?;
        self.commit_store.save(&self.commits)?;
        Ok(head)
    }

    /// Hard reset the current branch to `target` (a commit hash / prefix
    /// / `ROOT`). Preconditions: no running container, clean upper, HEAD
    /// on a branch.
    pub fn reset(&mut self, target: &str) -> Result<Hash, WorkspaceError> {
        self.ensure_not_running()?;
        self.ensure_clean()?;
        let hash = self.resolve_commit(target)?;
        let commit = self
            .commits
            .find(&hash)
            .cloned()
            .ok_or_else(|| WorkspaceError::UnknownTarget(target.to_string()))?;
        Vcs::reset(&mut self.commits, &commit)?;
        self.commit_store.save(&self.commits)?;
        Ok(hash)
    }

    /// Create a branch at the current HEAD.
    pub fn branch_create(&mut self, name: &str) -> Result<(), WorkspaceError> {
        Vcs::branch_create(&mut self.commits, name)?;
        self.commit_store.save(&self.commits)?;
        Ok(())
    }

    /// Delete a branch (not the one HEAD is on).
    pub fn branch_delete(&mut self, name: &str) -> Result<(), WorkspaceError> {
        Vcs::branch_delete(&mut self.commits, name)?;
        self.commit_store.save(&self.commits)?;
        Ok(())
    }

    /// Rebase branch `target` onto branch `newbase` (pointer surgery
    /// only; no hash recomputation, no layer renames). Preconditions: no
    /// running container, clean upper.
    pub fn rebase(&mut self, newbase: &str, target: &str) -> Result<(), WorkspaceError> {
        self.ensure_not_running()?;
        self.ensure_clean()?;
        Vcs::rebase(&mut self.commits, newbase, target)?;
        self.commit_store.save(&self.commits)?;
        Ok(())
    }

    /// Discard the upper: remove `diff/` and recreate it empty.
    /// Precondition: no running container.
    pub fn clean(&mut self) -> Result<(), WorkspaceError> {
        self.ensure_not_running()?;
        let path = self.env.upper_path();
        std::fs::remove_dir_all(&path)?;
        std::fs::create_dir_all(&path)?;
        Ok(())
    }

    /// Garbage-collect: drop commits unreachable from any branch or HEAD
    /// and delete their layers. Precondition: no running container.
    /// Returns the deleted layer hashes.
    pub fn gc(&mut self) -> Result<Vec<Hash>, WorkspaceError> {
        self.ensure_not_running()?;
        let (cleaned, dead_layers) = Vcs::gc(std::mem::take(&mut self.commits));
        self.commits = cleaned;
        for layer in &dead_layers {
            self.layer_store.delete(layer)?;
        }
        self.commit_store.save(&self.commits)?;
        Ok(dead_layers)
    }

    /// Commit history of HEAD, newest first (rendering is the CLI's job).
    pub fn log(&self) -> Vec<&Commit> {
        self.commits.head().commits().collect()
    }

    /// Compute a diff (read-only; no lock needed).
    ///
    /// - `diff(None, _)`      — uncommitted changes: upper vs baseline.
    /// - `diff(Some(a), None)` — commit `a`'s cumulative changes vs base.
    /// - `diff(Some(a), Some(b))` — `a` vs `b` (both resolved like
    ///   checkout targets, ROOT included).
    pub fn diff(
        &self,
        a: Option<&str>,
        b: Option<&str>,
    ) -> Result<Vec<Change>, WorkspaceError> {
        let image = self.image()?;
        match (a, b) {
            (None, _) => {
                let new = vec![Layer::new(self.env.upper_path())];
                Ok(changes(&new, &image.baseline(), self.blacklist())?)
            }
            (Some(a), None) => {
                let new = self.stack_of(self.resolve_target_commit(a)?)?;
                Ok(changes(&new, &image.base_only(), self.blacklist())?)
            }
            (Some(a), Some(b)) => {
                let new = self.stack_of(self.resolve_target_commit(a)?)?;
                let base = self.stack_of(self.resolve_target_commit(b)?)?;
                Ok(changes(&new, &base, self.blacklist())?)
            }
        }
    }

    /// Resolve a diff target that may be a branch name, commit or ROOT.
    fn resolve_target_commit(&self, target: &str) -> Result<Hash, WorkspaceError> {
        if let Some(branch) = self.commits.branch(target) {
            return Ok(*branch.commit_hash());
        }
        self.resolve_commit(target)
    }

    // ----- apply -------------------------------------------------------

    /// Whether an unfinished apply journal exists.
    pub fn apply_pending(&self) -> bool {
        apply::has_pending(&self.env.apply_journal_path())
    }

    /// Compute (and optionally execute) the host application of this
    /// environment's changes. Host-based only.
    ///
    /// `no_upper` excludes uncommitted changes; `dry_run` only computes;
    /// otherwise the changes are executed under the journal in
    /// `apply-journal/`. The confirmation prompt lives in the CLI —
    /// `force` merely asserts it already happened.
    pub fn apply(
        &self,
        no_upper: bool,
        force: bool,
        dry_run: bool,
    ) -> Result<Vec<Change>, WorkspaceError> {
        if !matches!(self.env.base_ref, BaseImageRef::Host) {
            return Err(WorkspaceError::HostOnly);
        }
        if self.apply_pending() {
            return Err(WorkspaceError::ApplyPending);
        }
        let head = self.commits.head().commit_hash();
        let mut new = Vec::new();
        if !no_upper {
            new.push(Layer::new(self.env.upper_path()));
        }
        new.extend(self.stack_of(head)?);
        let base = vec![Layer::new(std::path::PathBuf::from("/"))];
        let list = changes(&new, &base, self.blacklist())?;
        if dry_run || !force {
            return Ok(list);
        }
        self.ensure_not_running()?;
        apply::execute(&list, &self.env.apply_journal_path())?;
        Ok(list)
    }

    /// Roll back an unfinished apply journal.
    pub fn apply_rollback(&self) -> Result<(), WorkspaceError> {
        Ok(apply::rollback(&self.env.apply_journal_path())?)
    }

    /// Resume an unfinished apply journal.
    pub fn apply_resume(&self) -> Result<(), WorkspaceError> {
        Ok(apply::resume(&self.env.apply_journal_path())?)
    }

    /// The pending journal's manifest (for display before recovery).
    pub fn apply_pending_manifest(&self) -> Result<Vec<Change>, WorkspaceError> {
        Ok(apply::load_manifest(&self.env.apply_journal_path())?)
    }
}

/// Convenience used by the CLI: does `path` contain a setuid/setgid
/// source? Flagged during the apply confirmation.
pub fn is_setuid_source(change: &Change) -> bool {
    use std::os::unix::fs::PermissionsExt;
    change
        .source()
        .and_then(|s| s.symlink_metadata().ok())
        .is_some_and(|md| md.permissions().mode() & 0o6000 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvStore;
    use std::path::Path;

    /// Build a store + host env in a temp dir and write the initial
    /// commit graph, mimicking `orca init`.
    fn setup(dir: &Path) -> (EnvStore, uuid::Uuid) {
        let mut store = EnvStore::load(&dir.join("envs")).unwrap();
        let uuid = {
            let env = store.create("t".into(), BaseImageRef::Host).unwrap();
            let commits_file = env.env_path().join(COMMIT_FILE_NAME);
            CommitStore::new(&commits_file)
                .save(&CommitsData::new())
                .unwrap();
            env.uuid
        };
        store.save().unwrap();
        (store, uuid)
    }

    fn write_upper(env: &Env, rel: &str, content: &[u8]) {
        let path = env.upper_path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn commit_promotes_upper_and_advances_head() {
        let dir = tempfile::tempdir().unwrap();
        let (store, uuid) = setup(dir.path());
        let env = store.find_by_id(&uuid).unwrap();
        write_upper(env, "etc/foo", b"1");

        let mut ws = Workspace::open(env).unwrap();
        let hash = ws.commit("add foo").unwrap();
        // diff/ is empty again; the layer holds the file.
        assert!(std::fs::read_dir(env.upper_path()).unwrap().next().is_none());
        assert!(
            env.layers_path()
                .join(to_hex(&hash))
                .join("etc/foo")
                .is_file()
        );
        assert_eq!(ws.log().len(), 2);
        assert_eq!(ws.log()[0].message(), "add foo");
    }

    #[test]
    fn checkout_requires_clean_upper() {
        let dir = tempfile::tempdir().unwrap();
        let (store, uuid) = setup(dir.path());
        let env = store.find_by_id(&uuid).unwrap();
        let mut ws = Workspace::open(env).unwrap();
        ws.commit("c1").unwrap();

        write_upper(env, "dirty", b"x");
        assert!(matches!(
            ws.checkout("ROOT"),
            Err(WorkspaceError::NotClean)
        ));
        ws.clean().unwrap();
        let head = ws.checkout("ROOT").unwrap();
        assert!(matches!(head, Head::Detached(_)));
        // Back to a branch.
        ws.checkout("main").unwrap();
        assert_eq!(ws.data().head().branch_name(), Some("main"));
    }

    #[test]
    fn reset_and_gc_drop_unreachable_layer() {
        let dir = tempfile::tempdir().unwrap();
        let (store, uuid) = setup(dir.path());
        let env = store.find_by_id(&uuid).unwrap();
        let mut ws = Workspace::open(env).unwrap();
        write_upper(env, "a", b"1");
        let first = ws.commit("first").unwrap();
        write_upper(env, "b", b"2");
        let second = ws.commit("second").unwrap();

        ws.reset(&to_hex(&first)[..12]).unwrap();
        assert_eq!(ws.data().head().commit_hash(), first);

        let dead = ws.gc().unwrap();
        assert_eq!(dead, vec![second]);
        assert!(!env.layers_path().join(to_hex(&second)).exists());
        assert!(env.layers_path().join(to_hex(&first)).exists());
    }

    #[test]
    fn running_lock_blocks_operations() {
        let dir = tempfile::tempdir().unwrap();
        let (store, uuid) = setup(dir.path());
        let env = store.find_by_id(&uuid).unwrap();
        let lock = LockFile::acquire(&env.lock_path()).unwrap();

        let mut ws = Workspace::open(env).unwrap();
        assert!(matches!(
            ws.commit("x"),
            Err(WorkspaceError::Running(_))
        ));
        assert!(matches!(ws.clean(), Err(WorkspaceError::Running(_))));
        lock.release().unwrap();
        ws.commit("x").unwrap();
    }

    /// Build a *guest* env whose base is a fabricated image with one
    /// empty layer inside the tempdir. Unlike a host env (whose baseline
    /// includes the real `/`), every path a diff consults lives in the
    /// tempdir, keeping the test environment-independent.
    fn setup_guest(dir: &Path) -> (EnvStore, uuid::Uuid) {
        use orca_image::external_image::{ImageDigest, ImageManifest, LayerDigest};

        let digest = ImageDigest([9; 32]);
        let layer = LayerDigest([8; 32]);
        let mut store = EnvStore::load(&dir.join("envs")).unwrap();
        let uuid = {
            let env = store
                .create(
                    "g".into(),
                    BaseImageRef::External {
                        image_digest: digest,
                    },
                )
                .unwrap();
            let commits_file = env.env_path().join(COMMIT_FILE_NAME);
            CommitStore::new(&commits_file)
                .save(&CommitsData::new())
                .unwrap();
            std::fs::create_dir_all(env.blob_store_path().join(layer.to_string())).unwrap();
            let mut index = ImageIndex::load(&env.images_path()).unwrap();
            index.insert(ImageManifest {
                digest,
                registry: "docker.io".into(),
                repository: "library/fake".into(),
                tag: "1".into(),
                layer_digests: vec![layer],
                entrypoint: vec![],
                cmd: vec![],
                env: vec![],
                working_dir: "/".into(),
                pulled_at: chrono::Utc::now(),
            });
            index.save().unwrap();
            env.uuid
        };
        store.save().unwrap();
        (store, uuid)
    }

    #[test]
    fn diff_reports_uncommitted_changes_against_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let (store, uuid) = setup_guest(dir.path());
        let env = store.find_by_id(&uuid).unwrap();
        let mut ws = Workspace::open(env).unwrap();

        // Commit a layer, then add an uncommitted file.
        write_upper(env, "committed", b"1");
        ws.commit("c1").unwrap();
        write_upper(env, "uncommitted", b"2");

        let ws = Workspace::open(env).unwrap();
        let list = ws.diff(None, None).unwrap();
        // The committed file is part of the baseline (lower layer), so
        // only the uncommitted upper content is reported.
        assert_eq!(
            list,
            vec![Change::Create {
                path: "uncommitted".into(),
                source: env.upper_path().join("uncommitted"),
            }]
        );
    }

    #[test]
    fn diff_respects_blacklist_from_settings() {
        let dir = tempfile::tempdir().unwrap();
        let (_, uuid) = setup_guest(dir.path());
        // Configure a [defaults] blacklist the way a user would: by
        // editing envs.toml, then reloading the store.
        let file = dir.path().join("envs/envs.toml");
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str("\n[defaults]\nblacklist = [\"*.secret\"]\n");
        std::fs::write(&file, &text).unwrap();
        let store = EnvStore::load(&dir.path().join("envs")).unwrap();
        let env = store.find_by_id(&uuid).unwrap();

        write_upper(env, "keep.txt", b"1");
        write_upper(env, "x.secret", b"2");
        let ws = Workspace::open(env).unwrap();
        let list = ws.diff(None, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path(), Path::new("keep.txt"));
    }

    #[test]
    fn apply_rejected_for_guest_and_when_dirty_journal() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = setup(dir.path());
        let guest = store
            .create(
                "g".into(),
                BaseImageRef::External {
                    image_digest: orca_image::external_image::ImageDigest([1; 32]),
                },
            )
            .unwrap();
        let commits_file = guest.env_path().join(COMMIT_FILE_NAME);
        CommitStore::new(&commits_file)
            .save(&CommitsData::new())
            .unwrap();
        let ws = Workspace::open(guest).unwrap();
        assert!(matches!(
            ws.apply(false, false, true),
            Err(WorkspaceError::HostOnly)
        ));
    }
}
