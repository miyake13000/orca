//! [`CommitStore`]: load/save of `commits.toml`.

use std::path::{Path, PathBuf};

use crate::{CommitsData, VcsError};

/// Persistence for [`CommitsData`], backed by `envs/<uuid>/commits.toml`.
pub struct CommitStore {
    path: PathBuf,
}

impl CommitStore {
    /// Create a store rooted at the environment directory
    /// (`envs/<uuid>/`); the file managed is `<base_path>/commits.toml`.
    pub fn new(base_path: &Path) -> Self {
        Self {
            path: base_path.join("commits.toml"),
        }
    }

    /// Load and parse `commits.toml`.
    ///
    /// Returns `Err` if the file is missing or malformed.
    pub fn load(&self) -> Result<CommitsData, VcsError> {
        let text = std::fs::read_to_string(&self.path)?;
        Ok(toml::from_str(&text)?)
    }

    /// Serialize and write `commits.toml`, creating parent directories.
    pub fn save(&self, data: &CommitsData) -> Result<(), VcsError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(data)?;
        std::fs::write(&self.path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = CommitStore::new(dir.path());
        let data = CommitsData::new();
        store.save(&data).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.root().hash(), data.root().hash());
        assert_eq!(loaded.head().branch_name(), Some("main"));
    }
}
