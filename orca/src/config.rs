//! User-owned configuration, independent of the environment index.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Configuration loading or validation failure.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Configuration could not be read.
    #[error("config {path}: {source}")]
    Io {
        /// Requested file.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// Invalid TOML or unknown field.
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
    /// A setting is not meaningful.
    #[error("invalid setting: {0}")]
    Invalid(String),
}

/// Optional namespace overrides; true means isolate, false means share.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NamespaceSettings {
    /// PID namespace (default true).
    pub pid: Option<bool>,
    /// UTS namespace (default true).
    pub uts: Option<bool>,
    /// IPC namespace (default true).
    pub ipc: Option<bool>,
    /// Network namespace (default false).
    pub network: Option<bool>,
}

/// Overrides shared by global, named-environment and one-shot settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Complete argv; no shell interpolation. An explicit empty list is invalid.
    pub cmd: Option<Vec<String>>,
    /// Literal environment values, merged by variable name.
    pub env: BTreeMap<String, String>,
    /// Absolute path inside the container.
    pub working_dir: Option<PathBuf>,
    /// User name or decimal UID in the container.
    pub user: Option<String>,
    /// Group name or decimal GID in the container.
    pub group: Option<String>,
    /// Namespace overrides.
    pub namespaces: NamespaceSettings,
    /// Ordered diff/apply exclusions, including ! exceptions.
    pub blacklist: Vec<String>,
}
impl Settings {
    /// Overlay `higher` on `self`: scalars/argv replace, env merges by key,
    /// exclusions append. Neither input is modified.
    pub fn merged(&self, higher: &Self) -> Self {
        let mut env = self.env.clone();
        env.extend(higher.env.clone());
        let mut blacklist = self.blacklist.clone();
        blacklist.extend(higher.blacklist.clone());
        Self {
            cmd: higher.cmd.clone().or_else(|| self.cmd.clone()),
            env,
            working_dir: higher
                .working_dir
                .clone()
                .or_else(|| self.working_dir.clone()),
            user: higher.user.clone().or_else(|| self.user.clone()),
            group: higher.group.clone().or_else(|| self.group.clone()),
            namespaces: NamespaceSettings {
                pid: higher.namespaces.pid.or(self.namespaces.pid),
                uts: higher.namespaces.uts.or(self.namespaces.uts),
                ipc: higher.namespaces.ipc.or(self.namespaces.ipc),
                network: higher.namespaces.network.or(self.namespaces.network),
            },
            blacklist,
        }
    }
    /// Validate syntax independent of the target filesystem. Names and cwd
    /// existence are checked only when the selected environment is started.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(cmd) = &self.cmd {
            if cmd.is_empty() || cmd[0].is_empty() || cmd.iter().any(|s| s.contains('\0')) {
                return Err(ConfigError::Invalid(
                    "cmd must be nonempty argv without NUL".into(),
                ));
            }
        }
        if let Some(cwd) = &self.working_dir {
            if !cwd.is_absolute() || cwd.as_os_str().as_encoded_bytes().contains(&0) {
                return Err(ConfigError::Invalid(
                    "working_dir must be an absolute path without NUL".into(),
                ));
            }
        }
        for (key, value) in &self.env {
            if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
                return Err(ConfigError::Invalid(format!(
                    "invalid environment variable {key:?}"
                )));
            }
        }
        for spec in [&self.user, &self.group].into_iter().flatten() {
            if spec.is_empty() || spec.contains([':', '\0']) {
                return Err(ConfigError::Invalid(
                    "user/group must be a nonempty name or ID".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Read-only user configuration. Environment keys are exact, case-sensitive names.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Shared overrides.
    pub global: Settings,
    /// Overrides by environment name; names need not exist yet.
    pub envs: BTreeMap<String, Settings>,
}
impl Config {
    /// Read a caller-supplied file. Missing means empty; all other I/O,
    /// syntax, type and unknown-field errors are reported. Never writes.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let config: Self = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.into(),
                    source,
                });
            }
        };
        config.global.validate()?;
        for settings in config.envs.values() {
            settings.validate()?;
        }
        Ok(config)
    }
    /// Resolve global < named environment < one-shot overrides.
    pub fn resolve(&self, name: &str, overrides: &Settings) -> Result<Settings, ConfigError> {
        let result = self
            .global
            .merged(self.envs.get(name).unwrap_or(&Settings::default()))
            .merged(overrides);
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_layers_merge_keys_and_preserve_false() {
        let c: Config = toml::from_str(
            r#"
[global]
cmd = ["bash"]
blacklist = ["*.log"]
[global.env]
A = "global"
B = "kept"
[global.namespaces]
network = true
[envs."dev.1"]
blacklist = ["!keep.log"]
[envs."dev.1".env]
A = "env"
[envs."dev.1".namespaces]
network = false
"#,
        )
        .unwrap();
        let mut args = Settings::default();
        args.env.insert("A".into(), "arg".into());
        let s = c.resolve("dev.1", &args).unwrap();
        assert_eq!(s.env["A"], "arg");
        assert_eq!(s.env["B"], "kept");
        assert_eq!(s.namespaces.network, Some(false));
        assert_eq!(s.blacklist, ["*.log", "!keep.log"]);
        assert_eq!(
            c.resolve("other", &Settings::default())
                .unwrap()
                .namespaces
                .network,
            Some(true)
        );
    }
    #[test]
    fn strict_fields_but_arbitrary_env_names_and_variables() {
        for text in [
            "unknown = 1",
            "[global]\ncommmand = []",
            "[envs.dev.namespaces]\nnet = true",
            "[global]\nuser = 1000",
        ] {
            assert!(toml::from_str::<Config>(text).is_err(), "{text}");
        }
        assert!(toml::from_str::<Config>("[envs.future.env]\nANY_NAME = 'value'").is_ok());
        assert!(
            Settings {
                cmd: Some(vec![]),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
