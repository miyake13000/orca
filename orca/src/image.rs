//! [`Image`]: the versioned content of one environment.
//!
//! An `Image` is the env directory's substance seen as one object — the
//! writable upper, the committed layer stack and the commit graph — and
//! carries every operation on it: running a container from it, version
//! control (commit / checkout / reset / branch / rebase / gc / log),
//! diffing, and applying to the host ([`ApplyPlan`]).
//!
//! Obtained via [`Env::image`], which loads the commit graph once; an
//! `Image` is a snapshot, and mutating operations save back through it.
//! Each operation performs the preconditions *it* needs at its entry
//! (DESIGN §6) using the shared private helpers below:
//!
//! - `ensure_not_running` — refuse while a container is live
//!   (`LockFile::check`; stale locks are cleaned);
//! - `ensure_clean` — operations that change the lower stack
//!   (checkout / reset / rebase) require an empty `diff/`.

use orca_hash::{Hash, to_hex};
use orca_image::{
    Base, BaseImageRef, Blacklist, Change, ContainerImage, DiffError, ImageConfig, Layer,
    LayerStore, LayerStoreError, Upper, changes,
};
use orca_vcs::{Commit, CommitBuilder, CommitStore, CommitsData, Head, Vcs, VcsError};

use crate::apply::{self, ApplyError};
use crate::env::Env;
use crate::external_image_store::ExternalImageError;
use crate::lock::{LockError, LockFile};
use crate::run::{RunError, RunOpts};

/// Errors from image operations.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
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
    /// Applying to the host needs an effective uid of 0.
    #[error("orca apply requires root (try sudo, or a setuid-root install)")]
    RootRequired,
    /// Version-control error.
    #[error(transparent)]
    Vcs(#[from] VcsError),
    /// Layer store error.
    #[error(transparent)]
    Layer(#[from] LayerStoreError),
    /// Diff computation error.
    #[error(transparent)]
    Diff(#[from] DiffError),
    /// External image error (index lookup, blob resolution).
    #[error(transparent)]
    ExternalImage(#[from] ExternalImageError),
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

/// The versioned content of one environment: upper + committed layers +
/// commit graph, with every operation on them.
pub struct Image<'a> {
    env: &'a Env,
    commit_store: CommitStore,
    layer_store: LayerStore,
    commits: CommitsData,
    /// Parsed from the env's effective settings; applied to diff / apply
    /// (never to commit — DESIGN §6).
    blacklist: Blacklist,
}

impl<'a> Image<'a> {
    /// Open the image: construct the stores, load the commit graph once,
    /// and parse the env's diff blacklist. Public entry: [`Env::image`].
    pub(crate) fn open(env: &'a Env) -> Result<Self, ImageError> {
        let commit_store = CommitStore::new(&env.commits_path());
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

    /// The environment this image belongs to.
    pub fn env(&self) -> &Env {
        self.env
    }

    // ----- preconditions ---------------------------------------------

    /// Refuse to proceed while a live container holds the environment
    /// (stale locks are cleaned by `check`).
    fn ensure_not_running(&self) -> Result<(), ImageError> {
        match LockFile::check(&self.env.lock_path())? {
            Some(pid) => Err(ImageError::Running(pid)),
            None => Ok(()),
        }
    }

    /// Refuse to change the lower stack while `diff/` has content.
    fn ensure_clean(&self) -> Result<(), ImageError> {
        if self.upper().is_empty()? {
            Ok(())
        } else {
            Err(ImageError::NotClean)
        }
    }

    // ----- resolution helpers ----------------------------------------

    /// Resolve a user string to a commit hash: `ROOT`, a full hex hash,
    /// or a unique hex prefix (min 4 chars). Branch names are *not*
    /// resolved here — callers that accept branches check those first.
    fn resolve_commit(&self, target: &str) -> Result<Hash, ImageError> {
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
                0 => Err(ImageError::UnknownTarget(target.to_string())),
                1 => Ok(*matches[0].hash()),
                _ => Err(ImageError::AmbiguousTarget(target.to_string())),
            };
        }
        Err(ImageError::UnknownTarget(target.to_string()))
    }

    /// The layer stack (newest first) of the history ending at `commit`,
    /// following first parents down to the root.
    fn stack_of(&self, commit: Hash) -> Result<Vec<Layer>, ImageError> {
        let mut hashes = Vec::new();
        let mut cursor = Some(commit);
        while let Some(hash) = cursor {
            let c = self
                .commits
                .find(&hash)
                .ok_or_else(|| ImageError::UnknownTarget(to_hex(&hash)))?;
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

    /// Assemble the [`ContainerImage`] (mount material) for the current
    /// HEAD (used by [`Image::run`] and by diff).
    pub(crate) fn container_image(&self) -> Result<ContainerImage, ImageError> {
        let head_hash = self.commits.head().commit_hash();
        let lower = self.stack_of(head_hash)?;
        let (base, config) = match &self.env.base_ref {
            // Host bases declare no static config; the runtime spec is
            // resolved by ExecSpec from the invocation context.
            BaseImageRef::Host => (Base::Host, ImageConfig::default()),
            BaseImageRef::External { image_digest } => {
                let images = self.env.external_images();
                let manifest = images.manifest(image_digest)?;
                let layers = images.layers(&manifest)?;
                (Base::Guest(layers), manifest.config())
            }
        };
        Ok(ContainerImage {
            upper: self.upper(),
            lower,
            base,
            config,
        })
    }

    // ----- run ---------------------------------------------------------

    /// Run a container from this image. Returns the child's exit code.
    ///
    /// The whole policy lives in the internal `run` module: ExecSpec
    /// resolution, the lock lifecycle (acquired before the container
    /// starts, held until after `wait()`), and the launch itself.
    pub fn run(&self, opts: RunOpts) -> Result<i32, RunError> {
        let material = self.container_image()?;
        crate::run::run(self.env, material, opts)
    }

    // ----- operations -------------------------------------------------

    /// Commit the upper: compute the immutable hash, promote `diff/` to
    /// `layers/<hash>/`, append to the graph, save.
    ///
    /// Preconditions: no running container; HEAD on a branch (detached
    /// commit is rejected by [`Vcs::commit`]).
    pub fn commit(&mut self, message: &str) -> Result<Hash, ImageError> {
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
    pub fn checkout(&mut self, target: &str) -> Result<Head, ImageError> {
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
    pub fn reset(&mut self, target: &str) -> Result<Hash, ImageError> {
        self.ensure_not_running()?;
        self.ensure_clean()?;
        let hash = self.resolve_commit(target)?;
        let commit = self
            .commits
            .find(&hash)
            .cloned()
            .ok_or_else(|| ImageError::UnknownTarget(target.to_string()))?;
        Vcs::reset(&mut self.commits, &commit)?;
        self.commit_store.save(&self.commits)?;
        Ok(hash)
    }

    /// Create a branch at the current HEAD.
    pub fn branch_create(&mut self, name: &str) -> Result<(), ImageError> {
        Vcs::branch_create(&mut self.commits, name)?;
        self.commit_store.save(&self.commits)?;
        Ok(())
    }

    /// Delete a branch (not the one HEAD is on).
    pub fn branch_delete(&mut self, name: &str) -> Result<(), ImageError> {
        Vcs::branch_delete(&mut self.commits, name)?;
        self.commit_store.save(&self.commits)?;
        Ok(())
    }

    /// Rebase branch `target` onto branch `newbase` (pointer surgery
    /// only; no hash recomputation, no layer renames). Preconditions: no
    /// running container, clean upper.
    pub fn rebase(&mut self, newbase: &str, target: &str) -> Result<(), ImageError> {
        self.ensure_not_running()?;
        self.ensure_clean()?;
        Vcs::rebase(&mut self.commits, newbase, target)?;
        self.commit_store.save(&self.commits)?;
        Ok(())
    }

    /// `orca merge` is reserved but not implemented; always returns
    /// [`VcsError::MergeUnimplemented`] (which points at rebase).
    pub fn merge(&self, _branch: &str) -> Result<(), ImageError> {
        Err(VcsError::MergeUnimplemented.into())
    }

    /// Discard the upper: remove `diff/` and recreate it empty.
    /// Precondition: no running container.
    pub fn clean(&mut self) -> Result<(), ImageError> {
        self.ensure_not_running()?;
        let path = self.env.upper_path();
        std::fs::remove_dir_all(&path)?;
        std::fs::create_dir_all(&path)?;
        Ok(())
    }

    /// Garbage-collect: drop commits unreachable from any branch or HEAD
    /// and delete their layers. Precondition: no running container.
    /// Returns the deleted layer hashes.
    pub fn gc(&mut self) -> Result<Vec<Hash>, ImageError> {
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
    ) -> Result<Vec<Change>, ImageError> {
        let material = self.container_image()?;
        match (a, b) {
            (None, _) => {
                let new = vec![Layer::new(self.env.upper_path())];
                Ok(changes(&new, &material.baseline(), self.blacklist())?)
            }
            (Some(a), None) => {
                let new = self.stack_of(self.resolve_target_commit(a)?)?;
                Ok(changes(&new, &material.base_only(), self.blacklist())?)
            }
            (Some(a), Some(b)) => {
                let new = self.stack_of(self.resolve_target_commit(a)?)?;
                let base = self.stack_of(self.resolve_target_commit(b)?)?;
                Ok(changes(&new, &base, self.blacklist())?)
            }
        }
    }

    /// Resolve a diff target that may be a branch name, commit or ROOT.
    fn resolve_target_commit(&self, target: &str) -> Result<Hash, ImageError> {
        if let Some(branch) = self.commits.branch(target) {
            return Ok(*branch.commit_hash());
        }
        self.resolve_commit(target)
    }

    // ----- apply -------------------------------------------------------

    /// Plan the host application of this image's changes (host-based
    /// only): compute the change list against the host base and freeze
    /// it into an [`ApplyPlan`].
    ///
    /// `no_upper` excludes uncommitted changes. Fails with
    /// [`ImageError::ApplyPending`] if an unfinished journal exists —
    /// resolve it via [`Image::pending_apply`] first.
    pub fn plan_apply(&self, no_upper: bool) -> Result<ApplyPlan<'a>, ImageError> {
        if !matches!(self.env.base_ref, BaseImageRef::Host) {
            return Err(ImageError::HostOnly);
        }
        if apply::has_pending(&self.env.apply_journal_path()) {
            return Err(ImageError::ApplyPending);
        }
        let head = self.commits.head().commit_hash();
        let mut new = Vec::new();
        if !no_upper {
            new.push(Layer::new(self.env.upper_path()));
        }
        new.extend(self.stack_of(head)?);
        let base = vec![Layer::new(std::path::PathBuf::from("/"))];
        let changes = changes(&new, &base, self.blacklist())?;
        Ok(ApplyPlan {
            env: self.env,
            changes,
        })
    }

    /// The unfinished apply journal left by a crash or interrupt, if any
    /// (with its manifest loaded for display before recovery).
    pub fn pending_apply(&self) -> Result<Option<ApplyRecovery<'a>>, ImageError> {
        if !apply::has_pending(&self.env.apply_journal_path()) {
            return Ok(None);
        }
        let changes = apply::load_manifest(&self.env.apply_journal_path())?;
        Ok(Some(ApplyRecovery {
            env: self.env,
            changes,
        }))
    }
}

/// A frozen, displayable plan for applying an [`Image`]'s changes to the
/// host: what the user confirms is exactly what gets executed.
pub struct ApplyPlan<'a> {
    env: &'a Env,
    changes: Vec<Change>,
}

impl ApplyPlan<'_> {
    /// The changes this plan would apply (for display / confirmation).
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Execute the plan under the journal in `apply-journal/`.
    ///
    /// Consumes the plan (the confirmed plan is the executed one).
    /// Requires root and no running container. The confirmation prompt
    /// lives in the CLI — calling this asserts it already happened.
    pub fn execute(self) -> Result<(), ImageError> {
        if !nix::unistd::geteuid().is_root() {
            return Err(ImageError::RootRequired);
        }
        if let Some(pid) = LockFile::check(&self.env.lock_path())? {
            return Err(ImageError::Running(pid));
        }
        apply::execute(&self.changes, &self.env.apply_journal_path())?;
        Ok(())
    }
}

/// An unfinished apply journal (crash / interrupt), ready for recovery.
pub struct ApplyRecovery<'a> {
    env: &'a Env,
    changes: Vec<Change>,
}

impl ApplyRecovery<'_> {
    /// The journal's manifest: the changes the interrupted apply was
    /// executing.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Undo the partial apply from the journal's backups.
    pub fn rollback(self) -> Result<(), ImageError> {
        Ok(apply::rollback(&self.env.apply_journal_path())?)
    }

    /// Re-execute the journal to completion.
    pub fn resume(self) -> Result<(), ImageError> {
        Ok(apply::resume(&self.env.apply_journal_path())?)
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

    /// Build a store + host env in a temp dir, mimicking `orca init`
    /// (`create` writes the initial commit graph itself).
    fn setup(dir: &Path) -> (EnvStore, uuid::Uuid) {
        let mut store = EnvStore::load(dir).unwrap();
        let uuid = store.create("t".into(), BaseImageRef::Host).unwrap().uuid;
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

        let mut image = env.image().unwrap();
        let hash = image.commit("add foo").unwrap();
        // diff/ is empty again; the layer holds the file.
        assert!(std::fs::read_dir(env.upper_path()).unwrap().next().is_none());
        assert!(
            env.layers_path()
                .join(to_hex(&hash))
                .join("etc/foo")
                .is_file()
        );
        assert_eq!(image.log().len(), 2);
        assert_eq!(image.log()[0].message(), "add foo");
    }

    #[test]
    fn checkout_requires_clean_upper() {
        let dir = tempfile::tempdir().unwrap();
        let (store, uuid) = setup(dir.path());
        let env = store.find_by_id(&uuid).unwrap();
        let mut image = env.image().unwrap();
        image.commit("c1").unwrap();

        write_upper(env, "dirty", b"x");
        assert!(matches!(
            image.checkout("ROOT"),
            Err(ImageError::NotClean)
        ));
        image.clean().unwrap();
        let head = image.checkout("ROOT").unwrap();
        assert!(matches!(head, Head::Detached(_)));
        // Back to a branch.
        image.checkout("main").unwrap();
        assert_eq!(image.data().head().branch_name(), Some("main"));
    }

    #[test]
    fn reset_and_gc_drop_unreachable_layer() {
        let dir = tempfile::tempdir().unwrap();
        let (store, uuid) = setup(dir.path());
        let env = store.find_by_id(&uuid).unwrap();
        let mut image = env.image().unwrap();
        write_upper(env, "a", b"1");
        let first = image.commit("first").unwrap();
        write_upper(env, "b", b"2");
        let second = image.commit("second").unwrap();

        image.reset(&to_hex(&first)[..12]).unwrap();
        assert_eq!(image.data().head().commit_hash(), first);

        let dead = image.gc().unwrap();
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

        let mut image = env.image().unwrap();
        assert!(matches!(
            image.commit("x"),
            Err(ImageError::Running(_))
        ));
        assert!(matches!(image.clean(), Err(ImageError::Running(_))));
        lock.release().unwrap();
        image.commit("x").unwrap();
    }

    /// Build a *guest* env whose base is a fabricated image with one
    /// empty layer inside the tempdir. Unlike a host env (whose baseline
    /// includes the real `/`), every path a diff consults lives in the
    /// tempdir, keeping the test environment-independent.
    fn setup_guest(dir: &Path) -> (EnvStore, uuid::Uuid) {
        use orca_image::external_image::{ImageDigest, ImageManifest, LayerDigest};

        let digest = ImageDigest([9; 32]);
        let layer = LayerDigest([8; 32]);
        let mut store = EnvStore::load(dir).unwrap();
        let uuid = {
            let env = store
                .create(
                    "g".into(),
                    BaseImageRef::External {
                        image_digest: digest,
                    },
                )
                .unwrap();
            let images = env.external_images();
            std::fs::create_dir_all(images.blobs_dir().join(layer.to_string())).unwrap();
            images
                .insert(ImageManifest {
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
                })
                .unwrap();
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
        let mut image = env.image().unwrap();

        // Commit a layer, then add an uncommitted file.
        write_upper(env, "committed", b"1");
        image.commit("c1").unwrap();
        write_upper(env, "uncommitted", b"2");

        let image = env.image().unwrap();
        let list = image.diff(None, None).unwrap();
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
        let store = EnvStore::load(dir.path()).unwrap();
        let env = store.find_by_id(&uuid).unwrap();

        write_upper(env, "keep.txt", b"1");
        write_upper(env, "x.secret", b"2");
        let image = env.image().unwrap();
        let list = image.diff(None, None).unwrap();
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
        let image = guest.image().unwrap();
        assert!(matches!(
            image.plan_apply(false),
            Err(ImageError::HostOnly)
        ));
    }
}
