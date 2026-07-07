//! [`ExecSpec`]: resolution of argv / env / cwd / run_as (policy).
//!
//! The three-layer split (DESIGN §5): `orca-image`'s `ImageConfig` is
//! static image data, `orca-container` is mechanism that executes resolved
//! values, and this module is the policy in between. It turns the image
//! declaration plus the *invocation context* (process environment, cwd,
//! real uid, passwd entries) and CLI flags into a concrete execution
//! specification.
//!
//! Host-based environments aim to reproduce "the shell you would normally
//! open": full environment inheritance, the target user's login shell,
//! and the current working directory. Guest environments follow the image
//! declaration. [`ExecSpec::resolve`] is a pure function — everything it
//! consumes arrives via [`Invocation`] / [`UserIdentity`] values — so the
//! sudo / setuid / flag combinations are unit-testable.

use std::path::PathBuf;

use orca_container::RunAs;
use orca_image::ImageConfig;

/// Errors from execution-spec resolution.
#[derive(Debug, thiserror::Error)]
pub enum ExecSpecError {
    /// `--user` named a user that does not exist on the host.
    #[error("user not found: {0}")]
    UserNotFound(String),
    /// `--group` named a group that does not exist on the host.
    #[error("group not found: {0}")]
    GroupNotFound(String),
    /// passwd/group database access failed.
    #[error("failed to read user database: {0}")]
    Lookup(#[from] nix::Error),
    /// The invocation context could not be captured.
    #[error("failed to capture invocation context: {0}")]
    Io(#[from] std::io::Error),
}

/// A user identity resolved from the host passwd/group database: the
/// candidate the container drops to, and the source of the identity
/// environment variables (`HOME` / `USER` / `LOGNAME` / `SHELL`).
#[derive(Debug, Clone)]
pub struct UserIdentity {
    /// uid.
    pub uid: u32,
    /// Primary gid (may be overridden by `--group`).
    pub gid: u32,
    /// Supplementary groups (`getgrouplist`).
    pub groups: Vec<u32>,
    /// Login name.
    pub name: String,
    /// Home directory (`pw_dir`).
    pub home: PathBuf,
    /// Login shell (`pw_shell`).
    pub shell: PathBuf,
}

impl UserIdentity {
    /// Look up an identity by uid.
    pub fn from_uid(uid: u32) -> Result<Self, ExecSpecError> {
        let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))?
            .ok_or_else(|| ExecSpecError::UserNotFound(uid.to_string()))?;
        Self::from_user(user)
    }

    /// Look up an identity by login name.
    pub fn from_name(name: &str) -> Result<Self, ExecSpecError> {
        let user = nix::unistd::User::from_name(name)?
            .ok_or_else(|| ExecSpecError::UserNotFound(name.to_string()))?;
        Self::from_user(user)
    }

    fn from_user(user: nix::unistd::User) -> Result<Self, ExecSpecError> {
        let cname = std::ffi::CString::new(user.name.clone())
            .map_err(|_| ExecSpecError::UserNotFound(user.name.clone()))?;
        // Supplementary groups; fall back to just the primary gid if the
        // group database is unavailable.
        let groups = nix::unistd::getgrouplist(&cname, user.gid)
            .map(|gs| gs.iter().map(|g| g.as_raw()).collect())
            .unwrap_or_else(|_| vec![user.gid.as_raw()]);
        Ok(Self {
            uid: user.uid.as_raw(),
            gid: user.gid.as_raw(),
            groups,
            name: user.name,
            home: user.dir,
            shell: user.shell,
        })
    }

    /// The [`RunAs`] mechanism input for this identity.
    pub fn run_as(&self) -> RunAs {
        RunAs {
            uid: self.uid,
            gid: self.gid,
            groups: self.groups.clone(),
        }
    }
}

/// Snapshot of the orca process's invocation context. Fabricate one in
/// tests to exercise [`ExecSpec::resolve`] without touching the real
/// environment.
#[derive(Debug, Clone)]
pub struct Invocation {
    /// The full process environment as `KEY=VALUE`.
    pub environ: Vec<String>,
    /// The process working directory.
    pub cwd: PathBuf,
    /// Whether the effective uid is root (setup capability).
    pub euid_is_root: bool,
    /// The real uid — differs from 0 under a setuid-root install invoked
    /// by a regular user.
    pub real_uid: u32,
}

impl Invocation {
    /// Capture the current process context.
    pub fn capture() -> Result<Self, ExecSpecError> {
        Ok(Self {
            environ: std::env::vars()
                .map(|(k, v)| format!("{k}={v}"))
                .collect(),
            cwd: std::env::current_dir()?,
            euid_is_root: nix::unistd::geteuid().is_root(),
            real_uid: nix::unistd::getuid().as_raw(),
        })
    }
}

/// Decide who the container should run as (impure: consults passwd/group).
///
/// Priority (DESIGN §4):
/// 1. `--user` / `--group` (uid/gid numbers or names); `--group` alone
///    applies to the otherwise-selected identity.
/// 2. setuid invocation (euid root, real uid not root): the real user.
/// 3. otherwise: `None` — stay root.
pub fn resolve_run_target(
    inv: &Invocation,
    user: Option<&str>,
    group: Option<&str>,
) -> Result<Option<UserIdentity>, ExecSpecError> {
    let mut target = match user {
        Some(spec) => Some(match spec.parse::<u32>() {
            Ok(uid) => UserIdentity::from_uid(uid)?,
            Err(_) => UserIdentity::from_name(spec)?,
        }),
        None if inv.euid_is_root && inv.real_uid != 0 => {
            Some(UserIdentity::from_uid(inv.real_uid)?)
        }
        None => match group {
            // --group alone: keep the current (real) identity, change gid.
            Some(_) => Some(UserIdentity::from_uid(inv.real_uid)?),
            None => None,
        },
    };
    if let (Some(target), Some(spec)) = (&mut target, group) {
        let gid = match spec.parse::<u32>() {
            Ok(gid) => gid,
            Err(_) => nix::unistd::Group::from_name(spec)?
                .ok_or_else(|| ExecSpecError::GroupNotFound(spec.to_string()))?
                .gid
                .as_raw(),
        };
        target.gid = gid;
    }
    Ok(target)
}

/// The fully resolved execution specification, handed verbatim to
/// `ContainerBuilder`.
#[derive(Debug)]
pub struct ExecSpec {
    /// Command and arguments.
    pub argv: Vec<String>,
    /// Environment (`KEY=VALUE`).
    pub env: Vec<String>,
    /// Working directory (the container falls back to `/` if absent).
    pub cwd: PathBuf,
    /// Identity to drop to before exec (`None` = stay root).
    pub run_as: Option<RunAs>,
}

impl ExecSpec {
    /// Resolve the execution spec. Pure — all context is passed in.
    ///
    /// Rules (DESIGN §5):
    /// - argv: `user_cmd` > (host) `target.shell` > `$SHELL` > `/bin/bash`
    ///   / (guest) `entrypoint + cmd`.
    /// - env: (host) inherit `inv.environ`, overriding `HOME` `USER`
    ///   `LOGNAME` `SHELL` from `target` when dropping (this cancels
    ///   sudo's env_reset so the sudo and setuid paths agree) / (guest)
    ///   the image env. A missing `TERM` is supplemented in tty mode.
    /// - cwd: (host) `inv.cwd` / (guest) `config.working_dir`.
    pub fn resolve(
        is_host: bool,
        config: &ImageConfig,
        inv: &Invocation,
        target: Option<&UserIdentity>,
        user_cmd: Option<Vec<String>>,
        tty: bool,
    ) -> ExecSpec {
        // --- env ---
        let mut env: Vec<String> = if is_host {
            inv.environ.clone()
        } else {
            config.env.clone()
        };
        if is_host && let Some(user) = target {
            set_env(&mut env, "HOME", &user.home.to_string_lossy());
            set_env(&mut env, "USER", &user.name);
            set_env(&mut env, "LOGNAME", &user.name);
            set_env(&mut env, "SHELL", &user.shell.to_string_lossy());
        }
        if tty && get_env(&env, "TERM").is_none() {
            let term = get_env(&inv.environ, "TERM").unwrap_or("xterm");
            let term = format!("TERM={term}");
            env.push(term);
        }

        // --- argv ---
        let argv = match user_cmd {
            Some(cmd) if !cmd.is_empty() => cmd,
            _ if is_host => {
                let shell = target
                    .map(|u| u.shell.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .or_else(|| get_env(&inv.environ, "SHELL").map(str::to_string))
                    .unwrap_or_else(|| "/bin/bash".to_string());
                vec![shell]
            }
            _ => {
                let mut argv = config.entrypoint.clone();
                argv.extend(config.cmd.clone());
                argv
            }
        };

        // --- cwd ---
        let cwd = if is_host {
            inv.cwd.clone()
        } else if config.working_dir.as_os_str().is_empty() {
            PathBuf::from("/")
        } else {
            config.working_dir.clone()
        };

        ExecSpec {
            argv,
            env,
            cwd,
            run_as: target.map(UserIdentity::run_as),
        }
    }
}

/// Get `KEY`'s value from a `KEY=VALUE` list.
fn get_env<'a>(env: &'a [String], key: &str) -> Option<&'a str> {
    env.iter()
        .find_map(|e| e.strip_prefix(key)?.strip_prefix('='))
}

/// Set (replace or append) `KEY=value` in a `KEY=VALUE` list.
fn set_env(env: &mut Vec<String>, key: &str, value: &str) {
    let entry = format!("{key}={value}");
    match env.iter_mut().find(|e| {
        e.split_once('=').map(|(k, _)| k) == Some(key)
    }) {
        Some(slot) => *slot = entry,
        None => env.push(entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fabricated identity — nothing here comes from the real host
    /// passwd; resolve() is pure, so these tests behave identically on
    /// any machine.
    fn identity() -> UserIdentity {
        UserIdentity {
            uid: 1234,
            gid: 5678,
            groups: vec![5678, 42],
            name: "testuser".into(),
            home: "/home/testuser".into(),
            shell: "/opt/fake/shell".into(),
        }
    }

    /// Fabricated sudo-like invocation: env reset to root's identity vars.
    fn sudo_invocation() -> Invocation {
        Invocation {
            environ: vec![
                "HOME=/root".into(),
                "USER=root".into(),
                "LOGNAME=root".into(),
                "SHELL=/bin/rootshell".into(),
                "PATH=/fake/bin".into(),
                "TERM=fake-term".into(),
            ],
            cwd: "/somewhere/work".into(),
            euid_is_root: true,
            real_uid: 0,
        }
    }

    #[test]
    fn host_with_target_overrides_identity_env_and_uses_login_shell() {
        let inv = sudo_invocation();
        let user = identity();
        let spec = ExecSpec::resolve(
            true,
            &ImageConfig::default(),
            &inv,
            Some(&user),
            None,
            true,
        );
        // sudo's env_reset is cancelled: identity vars come from the
        // (fabricated) passwd identity.
        assert!(spec.env.contains(&"HOME=/home/testuser".to_string()));
        assert!(spec.env.contains(&"USER=testuser".to_string()));
        assert!(spec.env.contains(&"LOGNAME=testuser".to_string()));
        assert!(spec.env.contains(&"SHELL=/opt/fake/shell".to_string()));
        // Non-identity env is inherited untouched.
        assert!(spec.env.contains(&"PATH=/fake/bin".to_string()));
        // Login shell, current cwd, full drop.
        assert_eq!(spec.argv, vec!["/opt/fake/shell"]);
        assert_eq!(spec.cwd, PathBuf::from("/somewhere/work"));
        let run_as = spec.run_as.expect("must drop");
        assert_eq!((run_as.uid, run_as.gid), (1234, 5678));
        assert_eq!(run_as.groups, vec![5678, 42]);
    }

    #[test]
    fn host_without_target_stays_root_and_inherits_everything() {
        let inv = sudo_invocation();
        let spec =
            ExecSpec::resolve(true, &ImageConfig::default(), &inv, None, None, false);
        assert!(spec.run_as.is_none());
        assert!(spec.env.contains(&"HOME=/root".to_string()));
        // Root's $SHELL (from the fabricated environ) is the default cmd.
        assert_eq!(spec.argv, vec!["/bin/rootshell"]);
    }

    #[test]
    fn explicit_cmd_wins() {
        let inv = sudo_invocation();
        let user = identity();
        let spec = ExecSpec::resolve(
            true,
            &ImageConfig::default(),
            &inv,
            Some(&user),
            Some(vec!["make".into(), "install".into()]),
            false,
        );
        assert_eq!(spec.argv, vec!["make", "install"]);
    }

    #[test]
    fn guest_follows_image_declaration() {
        let inv = sudo_invocation();
        let config = ImageConfig {
            entrypoint: vec!["/entry".into()],
            cmd: vec!["serve".into()],
            env: vec!["PATH=/bin".into()],
            working_dir: "/app".into(),
        };
        let user = identity();
        let spec = ExecSpec::resolve(false, &config, &inv, Some(&user), None, true);
        assert_eq!(spec.argv, vec!["/entry", "serve"]);
        // Image env, not the host environ; no identity override for guest
        // (the container's /etc/passwd is not the host's).
        assert_eq!(spec.env[0], "PATH=/bin");
        assert!(!spec.env.iter().any(|e| e.starts_with("HOME=")));
        // TERM supplemented from the invocation in tty mode.
        assert!(spec.env.contains(&"TERM=fake-term".to_string()));
        assert_eq!(spec.cwd, PathBuf::from("/app"));
        // The drop still applies on guests.
        assert!(spec.run_as.is_some());
    }

    #[test]
    fn term_falls_back_to_xterm() {
        let mut inv = sudo_invocation();
        inv.environ.retain(|e| !e.starts_with("TERM="));
        let config = ImageConfig::default();
        let spec = ExecSpec::resolve(false, &config, &inv, None, None, true);
        assert!(spec.env.contains(&"TERM=xterm".to_string()));
    }

    #[test]
    fn set_env_replaces_in_place() {
        let mut env = vec!["HOME=/root".to_string(), "PATH=/usr/bin".to_string()];
        set_env(&mut env, "HOME", "/home/x");
        set_env(&mut env, "NEW", "1");
        assert_eq!(env, vec!["HOME=/home/x", "PATH=/usr/bin", "NEW=1"]);
        assert_eq!(get_env(&env, "HOME"), Some("/home/x"));
        assert_eq!(get_env(&env, "HOM"), None);
    }
}
