//! [`CommitStore`]: load/save of the commits file.

use std::path::{Path, PathBuf};

use crate::{CommitsData, VcsError};

/// Persistence for [`CommitsData`], backed by a TOML file at a
/// caller-supplied path.
pub struct CommitStore {
    path: PathBuf,
}

impl CommitStore {
    /// Create a store managing the commits file at `file_path`.
    pub fn new(file_path: &Path) -> Self {
        Self {
            path: file_path.to_path_buf(),
        }
    }

    /// Load and parse the commits file.
    ///
    /// Returns `Err` if the file is missing or malformed.
    pub fn load(&self) -> Result<CommitsData, VcsError> {
        let text = std::fs::read_to_string(&self.path)?;
        Ok(toml::from_str(&text)?)
    }

    /// Serialize and write the commits file, creating parent directories.
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
        let file = dir.path().join("commits.toml");
        let store = CommitStore::new(&file);
        let data = CommitsData::new();
        store.save(&data).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.root().hash(), data.root().hash());
        assert_eq!(loaded.head().branch_name(), Some("main"));
    }
}
