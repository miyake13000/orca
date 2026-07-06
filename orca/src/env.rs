//! [`EnvStore`] / [`Env`]: environment CRUD and path resolution.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orca_image::BaseImageRef;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

/// One environment record plus its resolved filesystem locations.
///
/// Serialized into `envs.toml` (the `root` field is derived, not stored).
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
    /// orca data root; injected after deserialization.
    #[serde(skip)]
    root: PathBuf,
}

impl Env {
    /// The environment directory (`envs/<uuid>/`).
    pub fn env_path(&self) -> PathBuf {
        self.root.join("envs").join(self.uuid.to_string())
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

    /// The shared image area (`images/`).
    pub fn images_path(&self) -> PathBuf {
        self.root.join("images")
    }

    /// The extracted-blob CAS (`images/layers/sha256/`).
    pub fn blob_store_path(&self) -> PathBuf {
        self.images_path().join("layers").join("sha256")
    }
}

/// On-disk shape of `envs.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct EnvsFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<Uuid>,
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
    /// Load the store from the `envs/` directory (missing `envs.toml` =
    /// empty store). The orca root is derived as its parent.
    pub fn load(base_path: &Path) -> Result<Self, EnvError> {
        let root = base_path
            .parent()
            .unwrap_or(base_path)
            .to_path_buf();
        let file = base_path.join("envs.toml");
        let mut data: EnvsFile = match std::fs::read_to_string(&file) {
            Ok(text) => toml::from_str(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => EnvsFile::default(),
            Err(e) => return Err(e.into()),
        };
        for env in &mut data.envs {
            env.root = root.clone();
        }
        Ok(Self {
            root,
            envs_dir: base_path.to_path_buf(),
            data,
        })
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
    /// and creates `envs/<uuid>/diff/` and `envs/<uuid>/layers/`.
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
            root: self.root.clone(),
        };
        std::fs::create_dir_all(env.upper_path())?;
        std::fs::create_dir_all(env.layers_path())?;
        self.data.envs.push(env);
        Ok(self.data.envs.last().expect("just pushed"))
    }

    /// Delete an environment: removes `envs/<uuid>/` and `run/<uuid>/`
    /// and drops the record; unsets `current` if it pointed here.
    /// The caller must have verified no container is running.
    pub fn delete(&mut self, uuid: &Uuid) -> Result<(), EnvError> {
        let env = self.find_by_id(uuid)?;
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
        EnvStore::load(&dir.join("envs")).unwrap()
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
