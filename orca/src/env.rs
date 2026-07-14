//! [`EnvStore`] / [`Env`]: environment CRUD and path resolution.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orca_image::BaseImageRef;
use orca_vcs::{CommitStore, CommitsData};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::external_image_store::ExternalImageStore;
use crate::image::{Image, ImageError};
use crate::lock::{LockError, LockFile};

/// File name of the commit graph inside `envs/<uuid>/`.
const COMMIT_FILE_NAME: &str = "commits.toml";

/// Errors from environment management.
#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    /// No environment matches the given name or uuid.
    #[error("environment not found: {0}")]
    NotFound(String),
    /// An environment with this name already exists.
    #[error("environment already exists: {0}")]
    AlreadyExists(String),
    /// The environment name is not usable.
    #[error("invalid environment name: {0}")]
    InvalidName(String),
    /// No current environment is set (and neither --env nor ORCA_ENV was
    /// given).
    #[error("no environment selected; use `orca use <name>`, --env, or ORCA_ENV")]
    NoCurrent,
    /// envs.toml or an environment directory could not be accessed.
    #[error("environment store I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// envs.toml could not be parsed.
    #[error("failed to parse envs.toml: {0}")]
    Parse(#[from] toml::de::Error),
    /// envs.toml could not be serialized.
    #[error("failed to serialize envs.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// A live container holds the environment (delete refused).
    #[error("a container is running in {name} (pid {pid}); stop it first")]
    Running {
        /// The environment's name.
        name: String,
        /// The container's init pid on the host.
        pid: i32,
    },
    /// The run lock could not be inspected.
    #[error(transparent)]
    Lock(#[from] LockError),
    /// The initial commit graph could not be written.
    #[error("failed to initialize the commit graph: {0}")]
    Vcs(#[from] orca_vcs::VcsError),
}

/// The orca data root: `$ORCA_ROOT` if set (mainly for tests), otherwise
/// `$HOME/.local/share/orca`.
pub fn orca_root() -> PathBuf {
    if let Ok(root) = std::env::var("ORCA_ROOT") {
        return PathBuf::from(root);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.local/share"))
        .join("orca")
}

/// Optional per-environment configuration, used both for the global
/// `[defaults]` table and the per-env `[envs.settings]` table in
/// envs.toml.
///
/// The ImageConfig-shaped fields are `Option` (None = fall through to the
/// next source in the priority chain "CLI args > env-specific > defaults >
/// image declaration"); `blacklist` merges as a union instead. The
/// blacklist patterns themselves are interpreted by
/// `orca_image::Blacklist` (diff vocabulary) — this type only carries the
/// strings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnvSettings {
    /// Override for the image entrypoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
    /// Override for the image cmd (also the default command for
    /// host-based environments).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<Vec<String>>,
    /// Extra / overriding environment variables (`KEY=VALUE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
    /// Override for the working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<PathBuf>,
    /// Git-ignore-style patterns excluded from diff / apply (never from
    /// commit).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blacklist: Vec<String>,
}

impl EnvSettings {
    /// Merge `defaults` (the `[defaults]` table) with `specific` (the
    /// env's own `[envs.settings]`): specific `Some` fields replace
    /// wholesale, blacklists concatenate (union).
    pub fn merged(defaults: &EnvSettings, specific: &EnvSettings) -> EnvSettings {
        EnvSettings {
            entrypoint: specific
                .entrypoint
                .clone()
                .or_else(|| defaults.entrypoint.clone()),
            cmd: specific.cmd.clone().or_else(|| defaults.cmd.clone()),
            env: specific.env.clone().or_else(|| defaults.env.clone()),
            working_dir: specific
                .working_dir
                .clone()
                .or_else(|| defaults.working_dir.clone()),
            blacklist: defaults
                .blacklist
                .iter()
                .chain(specific.blacklist.iter())
                .cloned()
                .collect(),
        }
    }
}

/// One environment record plus its resolved filesystem locations.
///
/// Serialized into `envs.toml` (the `root` and `effective_settings`
/// fields are derived, not stored).
/// Invariant: `name` is non-empty and unique within the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Env {
    /// Stable identity; also the directory name under `envs/` and `run/`.
    pub uuid: Uuid,
    /// Required human-readable name (unique).
    pub name: String,
    /// What the environment is based on.
    #[serde(rename = "base")]
    pub base_ref: BaseImageRef,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Per-env configuration (`[envs.settings]`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<EnvSettings>,
    /// orca data root; injected after deserialization.
    #[serde(skip)]
    root: PathBuf,
    /// `[defaults]` merged with `settings`; computed on load/create.
    #[serde(skip)]
    effective_settings: EnvSettings,
}

impl Env {
    /// The effective configuration: `[defaults]` overlaid with this
    /// env's `[envs.settings]` (blacklist = union of both).
    pub fn settings(&self) -> &EnvSettings {
        &self.effective_settings
    }

    /// The environment directory (`envs/<uuid>/`).
    pub fn env_path(&self) -> PathBuf {
        self.root.join("envs").join(self.uuid.to_string())
    }

    /// The commit graph file (`envs/<uuid>/commits.toml`).
    pub fn commits_path(&self) -> PathBuf {
        self.env_path().join(COMMIT_FILE_NAME)
    }

    /// The writable upper (`envs/<uuid>/diff/`).
    pub fn upper_path(&self) -> PathBuf {
        self.env_path().join("diff")
    }

    /// Committed layers (`envs/<uuid>/layers/`).
    pub fn layers_path(&self) -> PathBuf {
        self.env_path().join("layers")
    }

    /// The apply journal directory (`envs/<uuid>/apply-journal/`),
    /// persistent so recovery survives crashes.
    pub fn apply_journal_path(&self) -> PathBuf {
        self.env_path().join("apply-journal")
    }

    /// The run lock (`run/<uuid>/lock`), owned by the orca lib; may exist
    /// while no session does.
    pub fn lock_path(&self) -> PathBuf {
        self.root.join("run").join(self.uuid.to_string()).join("lock")
    }

    /// The per-run session directory (`run/<uuid>/session/`), created and
    /// destroyed wholesale by `orca-container`.
    pub fn session_path(&self) -> PathBuf {
        self.root
            .join("run")
            .join(self.uuid.to_string())
            .join("session")
    }

    /// Overlay workdir (`run/<uuid>/session/work/`).
    pub fn work_path(&self) -> PathBuf {
        self.session_path().join("work")
    }

    /// Overlay mount point / pivot_root target
    /// (`run/<uuid>/session/rootfs/`).
    pub fn rootfs_path(&self) -> PathBuf {
        self.session_path().join("rootfs")
    }

    /// Host-based stage-1 overlay mount point
    /// (`run/<uuid>/session/fake_rootfs/`).
    pub fn fake_rootfs_path(&self) -> PathBuf {
        self.session_path().join("fake_rootfs")
    }

    /// Host-based stage-1 throwaway upper
    /// (`run/<uuid>/session/fake_upper/`).
    pub fn fake_upper_path(&self) -> PathBuf {
        self.session_path().join("fake_upper")
    }

    /// Host-based stage-1 workdir (`run/<uuid>/session/fake_work/`).
    pub fn fake_work_path(&self) -> PathBuf {
        self.session_path().join("fake_work")
    }

    /// The versioned content of this environment (loads the commit graph
    /// once; the returned [`Image`] is a snapshot of it).
    pub fn image(&self) -> Result<Image<'_>, ImageError> {
        Image::open(self)
    }

    /// The shared external-image area under the same data root. Internal:
    /// `base_ref` is a foreign key into it, and resolving one's own
    /// foreign key is this type's job.
    pub(crate) fn external_images(&self) -> ExternalImageStore {
        ExternalImageStore::new(&self.root)
    }
}

/// On-disk shape of `envs.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct EnvsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<Uuid>,
    /// Global `[defaults]` applied to every env (under its own settings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    defaults: Option<EnvSettings>,
    #[serde(default)]
    envs: Vec<Env>,
}

/// Manages `envs/`: the `envs.toml` index and environment directories.
pub struct EnvStore {
    root: PathBuf,
    envs_dir: PathBuf,
    data: EnvsFile,
}

impl EnvStore {
    /// Load the store from the data root (missing `envs.toml` = empty
    /// store). Public entry: [`crate::Orca::envs`].
    pub(crate) fn load(root: &Path) -> Result<Self, EnvError> {
        let root = root.to_path_buf();
        let base_path = root.join("envs");
        let file = base_path.join("envs.toml");
        let mut data: EnvsFile = match std::fs::read_to_string(&file) {
            Ok(text) => toml::from_str(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => EnvsFile::default(),
            Err(e) => return Err(e.into()),
        };
        let defaults = data.defaults.clone().unwrap_or_default();
        for env in &mut data.envs {
            env.root = root.clone();
            env.effective_settings =
                EnvSettings::merged(&defaults, env.settings.as_ref().unwrap_or(&Default::default()));
        }
        Ok(Self {
            root,
            envs_dir: base_path,
            data,
        })
    }

    /// The target environment: `Some(selector)` resolves a name or uuid,
    /// `None` falls back to the current environment.
    pub fn env(&self, selector: Option<&str>) -> Result<&Env, EnvError> {
        match selector {
            Some(target) => self.resolve(target),
            None => self.current(),
        }
    }

    /// Find an environment by name.
    pub fn find_by_name(&self, name: &str) -> Result<&Env, EnvError> {
        self.data
            .envs
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| EnvError::NotFound(name.to_string()))
    }

    /// Find an environment by uuid.
    pub fn find_by_id(&self, uuid: &Uuid) -> Result<&Env, EnvError> {
        self.data
            .envs
            .iter()
            .find(|e| &e.uuid == uuid)
            .ok_or_else(|| EnvError::NotFound(uuid.to_string()))
    }

    /// Resolve a user-supplied string: uuid first, then name.
    pub fn resolve(&self, target: &str) -> Result<&Env, EnvError> {
        if let Ok(uuid) = Uuid::parse_str(target)
            && let Ok(env) = self.find_by_id(&uuid)
        {
            return Ok(env);
        }
        self.find_by_name(target)
    }

    /// Create a new environment: validates the name, allocates a uuid,
    /// creates `envs/<uuid>/diff/` and `envs/<uuid>/layers/`, and writes
    /// the initial commit graph (an env without one would be broken —
    /// [`Env::image`] requires it).
    /// Does not touch `current` (the CLI decides) and does not save.
    pub fn create(&mut self, name: String, base_ref: BaseImageRef) -> Result<&Env, EnvError> {
        if name.is_empty() || name.contains('/') || name == "ROOT" {
            return Err(EnvError::InvalidName(name));
        }
        if Uuid::parse_str(&name).is_ok() {
            return Err(EnvError::InvalidName(name));
        }
        if self.data.envs.iter().any(|e| e.name == name) {
            return Err(EnvError::AlreadyExists(name));
        }
        let env = Env {
            uuid: Uuid::new_v4(),
            name,
            base_ref,
            created_at: Utc::now(),
            settings: None,
            root: self.root.clone(),
            effective_settings: self.data.defaults.clone().unwrap_or_default(),
        };
        std::fs::create_dir_all(env.upper_path())?;
        std::fs::create_dir_all(env.layers_path())?;
        CommitStore::new(&env.commits_path()).save(&CommitsData::new())?;
        self.data.envs.push(env);
        Ok(self.data.envs.last().expect("just pushed"))
    }

    /// Delete an environment: removes `envs/<uuid>/` and `run/<uuid>/`
    /// and drops the record; unsets `current` if it pointed here.
    /// Refuses while a container is running (stale locks are cleaned by
    /// the check).
    pub fn delete(&mut self, uuid: &Uuid) -> Result<(), EnvError> {
        let env = self.find_by_id(uuid)?;
        if let Some(pid) = LockFile::check(&env.lock_path())? {
            return Err(EnvError::Running {
                name: env.name.clone(),
                pid,
            });
        }
        let env_path = env.env_path();
        let run_path = self.root.join("run").join(uuid.to_string());
        for path in [env_path, run_path] {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.data.envs.retain(|e| &e.uuid != uuid);
        if self.data.current == Some(*uuid) {
            self.data.current = None;
        }
        Ok(())
    }

    /// All environments, in creation order.
    pub fn list(&self) -> Vec<&Env> {
        self.data.envs.iter().collect()
    }

    /// Set the current environment.
    pub fn set_current(&mut self, uuid: &Uuid) -> Result<(), EnvError> {
        self.find_by_id(uuid)?;
        self.data.current = Some(*uuid);
        Ok(())
    }

    /// The current environment ([`EnvError::NoCurrent`] if unset).
    pub fn current(&self) -> Result<&Env, EnvError> {
        let uuid = self.data.current.ok_or(EnvError::NoCurrent)?;
        self.find_by_id(&uuid)
    }

    /// The current environment's uuid, if set.
    pub fn current_uuid(&self) -> Option<Uuid> {
        self.data.current
    }

    /// Persist `envs.toml` (creates `envs/`).
    pub fn save(&self) -> Result<(), EnvError> {
        std::fs::create_dir_all(&self.envs_dir)?;
        let text = toml::to_string_pretty(&self.data)?;
        std::fs::write(self.envs_dir.join("envs.toml"), text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> EnvStore {
        EnvStore::load(dir).unwrap()
    }

    #[test]
    fn create_save_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        let uuid = {
            let env = s.create("myenv".into(), BaseImageRef::Host).unwrap();
            assert!(env.upper_path().is_dir());
            assert!(env.layers_path().is_dir());
            env.uuid
        };
        s.set_current(&uuid).unwrap();
        s.save().unwrap();

        let s2 = store(dir.path());
        assert_eq!(s2.current().unwrap().name, "myenv");
        assert_eq!(s2.resolve("myenv").unwrap().uuid, uuid);
        assert_eq!(s2.resolve(&uuid.to_string()).unwrap().name, "myenv");
        // Paths are rooted correctly after reload.
        assert!(s2.current().unwrap().upper_path().starts_with(dir.path()));

        let mut s3 = store(dir.path());
        s3.delete(&uuid).unwrap();
        assert!(s3.current().is_err());
        assert!(s3.resolve("myenv").is_err());
        s3.save().unwrap();
    }

    #[test]
    fn settings_merge_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.create("cfg".into(), BaseImageRef::Host).unwrap();
        s.save().unwrap();

        // Add per-env [envs.settings] and global [defaults] the way a
        // user would: by editing envs.toml. [envs.settings] attaches to
        // the most recent [[envs]] element.
        let file = dir.path().join("envs/envs.toml");
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str("\n[envs.settings]\ncmd = [\"/bin/zsh\"]\nblacklist = [\"/var/log\"]\n");
        text.push_str(
            "\n[defaults]\ncmd = [\"/bin/bash\"]\nenv = [\"EDITOR=vim\"]\nblacklist = [\"*.history\"]\n",
        );
        std::fs::write(&file, &text).unwrap();

        let s = store(dir.path());
        let env = s.find_by_name("cfg").unwrap();
        let settings = env.settings();
        // Specific replaces wholesale...
        assert_eq!(settings.cmd, Some(vec!["/bin/zsh".to_string()]));
        // ...unset fields fall through to defaults...
        assert_eq!(settings.env, Some(vec!["EDITOR=vim".to_string()]));
        assert_eq!(settings.entrypoint, None);
        // ...and blacklists union (defaults first).
        assert_eq!(settings.blacklist, vec!["*.history", "/var/log"]);

        // Saving keeps both tables intact.
        s.save().unwrap();
        let s = store(dir.path());
        let env = s.find_by_name("cfg").unwrap();
        assert_eq!(env.settings().blacklist, vec!["*.history", "/var/log"]);
    }

    #[test]
    fn merged_prefers_specific_and_unions_blacklist() {
        let defaults = EnvSettings {
            cmd: Some(vec!["a".into()]),
            working_dir: Some("/d".into()),
            blacklist: vec!["x".into()],
            ..Default::default()
        };
        let specific = EnvSettings {
            cmd: Some(vec!["b".into()]),
            blacklist: vec!["y".into()],
            ..Default::default()
        };
        let merged = EnvSettings::merged(&defaults, &specific);
        assert_eq!(merged.cmd, Some(vec!["b".to_string()]));
        assert_eq!(merged.working_dir, Some("/d".into()));
        assert_eq!(merged.blacklist, vec!["x", "y"]);
    }

    #[test]
    fn duplicate_and_invalid_names_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = store(dir.path());
        s.create("dup".into(), BaseImageRef::Host).unwrap();
        assert!(matches!(
            s.create("dup".into(), BaseImageRef::Host),
            Err(EnvError::AlreadyExists(_))
        ));
        assert!(matches!(
            s.create("".into(), BaseImageRef::Host),
            Err(EnvError::InvalidName(_))
        ));
        assert!(matches!(
            s.create("ROOT".into(), BaseImageRef::Host),
            Err(EnvError::InvalidName(_))
        ));
        assert!(matches!(
            s.create(Uuid::new_v4().to_string(), BaseImageRef::Host),
            Err(EnvError::InvalidName(_))
        ));
    }
}
