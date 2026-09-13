//! Execution policy finalized in the child rootfs, without NSS or host env inheritance.
use crate::Settings;
use orca_container::{Execution, RunAs};
use orca_image::ImageConfig;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Predictable executable search path when an image declares none.
const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Identity resolved solely from the target's passwd/group files.
#[derive(Debug)]
struct Identity {
    uid: u32,
    gid: u32,
    groups: Vec<u32>,
    name: String,
    home: String,
    shell: String,
}

/// Parse a numeric identifier, rejecting the syscall sentinel `(uid_t)-1`.
fn numeric(spec: &str) -> Result<Option<u32>, String> {
    if !spec.is_empty() && spec.bytes().all(|c| c.is_ascii_digit()) {
        let id = spec
            .parse::<u32>()
            .map_err(|_| format!("ID out of range: {spec}"))?;
        if id == u32::MAX {
            return Err(format!("reserved ID: {spec}"));
        }
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

/// Missing account files are permitted for numerical identities.
fn database(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Resolve an identity from supplied file contents; no host NSS is consulted.
fn identity(
    user: &str,
    group: Option<&str>,
    passwd: &str,
    groups: &str,
) -> Result<Identity, String> {
    let uid = numeric(user)?;
    let entry = passwd
        .lines()
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(|s| s.split(':').collect::<Vec<_>>())
        .find(|f| {
            f.len() >= 7
                && match uid {
                    Some(uid) => f[2].parse::<u32>().ok() == Some(uid),
                    None => f[0] == user,
                }
        });
    let mut result = match entry {
        Some(f) => Identity {
            uid: numeric(f[2])?.ok_or("invalid passwd UID")?,
            gid: numeric(f[3])?.ok_or("invalid passwd GID")?,
            groups: Vec::new(),
            name: f[0].into(),
            home: if f[5].is_empty() { "/" } else { f[5] }.into(),
            shell: if f[6].is_empty() { "/bin/sh" } else { f[6] }.into(),
        },
        None => Identity {
            uid: uid.ok_or_else(|| format!("user not found in /etc/passwd: {user}"))?,
            gid: 0,
            groups: Vec::new(),
            name: user.into(),
            home: "/".into(),
            shell: "/bin/sh".into(),
        },
    };
    if let Some(group) = group {
        result.gid = match numeric(group)? {
            Some(gid) => gid,
            None => {
                let f = groups
                    .lines()
                    .map(|s| s.split(':').collect::<Vec<_>>())
                    .find(|f| f.len() >= 4 && f[0] == group)
                    .ok_or_else(|| format!("group not found in /etc/group: {group}"))?;
                numeric(f[2])?.ok_or("invalid group GID")?
            }
        };
    } else if entry_exists(passwd, result.uid) {
        for f in groups.lines().map(|s| s.split(':').collect::<Vec<_>>()) {
            if f.len() >= 4 && f[3].split(',').any(|name| name == result.name) {
                let gid = numeric(f[2])?.ok_or("invalid group GID")?;
                if gid != result.gid && !result.groups.contains(&gid) {
                    result.groups.push(gid);
                }
            }
        }
    }
    Ok(result)
}

fn entry_exists(passwd: &str, uid: u32) -> bool {
    passwd.lines().any(|s| {
        let f: Vec<_> = s.split(':').collect();
        f.len() >= 7 && f[2].parse::<u32>().ok() == Some(uid)
    })
}

/// Resolve after pivot_root. Reads `/etc/passwd` and `/etc/group`, and returns
/// final argv/env/credentials. Errors abort startup before privilege changes.
pub(crate) fn resolve(
    settings: &Settings,
    image: &ImageConfig,
    host: bool,
    invocation_cwd: &Path,
    term: Option<&str>,
    tty: bool,
) -> Result<Execution, String> {
    resolve_with_databases(
        settings,
        image,
        host,
        invocation_cwd,
        term,
        tty,
        &database(Path::new("/etc/passwd"))?,
        &database(Path::new("/etc/group"))?,
    )
}

fn resolve_with_databases(
    settings: &Settings,
    image: &ImageConfig,
    host: bool,
    invocation_cwd: &Path,
    term: Option<&str>,
    tty: bool,
    passwd: &str,
    groups: &str,
) -> Result<Execution, String> {
    settings.validate().map_err(|e| e.to_string())?;
    let (image_user, image_group) = image
        .user
        .split_once(':')
        .map(|(u, g)| (u, Some(g)))
        .unwrap_or((&image.user, None));
    let user = settings
        .user
        .as_deref()
        .unwrap_or(if image_user.is_empty() {
            "0"
        } else {
            image_user
        });
    // An explicitly selected user replaces the image's complete USER declaration;
    // an independently specified group still wins.
    let group = settings.group.as_deref().or(if settings.user.is_none() {
        image_group
    } else {
        None
    });
    let identity = identity(user, group, passwd, groups)?;
    let mut env = BTreeMap::from([("PATH".to_string(), DEFAULT_PATH.to_string())]);
    for e in &image.env {
        let (key, value) = e
            .split_once('=')
            .ok_or_else(|| format!("invalid image environment: {e:?}"))?;
        env.insert(key.into(), value.into());
    }
    for (key, value) in [
        ("HOME", &identity.home),
        ("USER", &identity.name),
        ("LOGNAME", &identity.name),
        ("SHELL", &identity.shell),
    ] {
        env.insert(key.into(), value.clone());
    }
    if tty {
        env.insert(
            "TERM".into(),
            term.filter(|s| !s.is_empty()).unwrap_or("dumb").into(),
        );
    }
    env.extend(settings.env.clone());
    let cmd = settings.cmd.clone().unwrap_or_else(|| {
        if host {
            vec![identity.shell.clone()]
        } else {
            image.entrypoint.iter().chain(&image.cmd).cloned().collect()
        }
    });
    if cmd.is_empty() || cmd[0].is_empty() || cmd.iter().any(|s| s.contains('\0')) {
        return Err("no valid command to execute".into());
    }
    if env
        .iter()
        .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
    {
        return Err("invalid environment".into());
    }
    let cwd = settings.working_dir.clone().unwrap_or_else(|| {
        if host {
            invocation_cwd.into()
        } else if image.working_dir.as_os_str().is_empty() {
            PathBuf::from("/")
        } else {
            image.working_dir.clone()
        }
    });
    if !cwd.is_absolute() {
        return Err(format!(
            "working directory must be absolute: {}",
            cwd.display()
        ));
    }
    Ok(Execution {
        cmd,
        env: env.into_iter().map(|(k, v)| format!("{k}={v}")).collect(),
        cwd,
        fallback_cwd: host && settings.working_dir.is_none(),
        run_as: RunAs {
            uid: identity.uid,
            gid: identity.gid,
            groups: identity.groups,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const PASSWD: &str = "root:x:0:0::/root:/bin/sh\nalice:x:1001:1002::/home/alice:/bin/bash\n";
    const GROUP: &str = "users:x:1002:\nextra:x:40:alice\nother:x:50:\n";
    #[test]
    fn names_numbers_and_explicit_group() {
        let a = identity("alice", None, PASSWD, GROUP).unwrap();
        assert_eq!((a.uid, a.gid, a.groups), (1001, 1002, vec![40]));
        let a = identity("1001", Some("other"), PASSWD, GROUP).unwrap();
        assert_eq!((a.uid, a.gid, a.groups), (1001, 50, vec![]));
        let a = identity("9876", Some("5432"), "", "").unwrap();
        assert_eq!((a.uid, a.gid, a.home.as_str()), (9876, 5432, "/"));
        assert_eq!(identity("9876", None, "", "").unwrap().gid, 0);
        assert!(identity("missing", None, PASSWD, GROUP).is_err());
        assert!(identity("1001", Some("missing"), PASSWD, GROUP).is_err());
        assert!(identity("4294967295", None, "", "").is_err());
    }
    #[test]
    fn image_defaults_explicit_env_and_cwd() {
        let image = ImageConfig {
            user: "alice:other".into(),
            cmd: vec!["serve".into()],
            env: vec!["PATH=/app/bin".into(), "HOME=/image".into()],
            working_dir: "/app".into(),
            ..Default::default()
        };
        let mut s = Settings::default();
        s.env.insert("HOME".into(), "/override".into());
        s.env.insert("TERM".into(), "explicit".into());
        let e = resolve_with_databases(
            &s,
            &image,
            false,
            Path::new("/host"),
            Some("host-term"),
            true,
            PASSWD,
            GROUP,
        )
        .unwrap();
        assert_eq!(e.run_as.gid, 50);
        assert!(e.run_as.groups.is_empty());
        assert!(e.env.contains(&"HOME=/override".into()));
        assert!(e.env.contains(&"PATH=/app/bin".into()));
        assert!(e.env.contains(&"TERM=explicit".into()));
        assert_eq!(e.cwd, Path::new("/app"));
        assert!(!e.fallback_cwd);
        s.user = Some("9876".into());
        let e = resolve_with_databases(
            &s,
            &image,
            false,
            Path::new("/host"),
            None,
            false,
            PASSWD,
            GROUP,
        )
        .unwrap();
        assert_eq!(e.run_as.gid, 0);
    }
    #[test]
    fn host_has_fixed_identity_path_and_only_terminal_inheritance() {
        let s = Settings::default();
        let image = ImageConfig::default();
        let e = resolve_with_databases(
            &s,
            &image,
            true,
            Path::new("/work"),
            Some("vt100"),
            true,
            PASSWD,
            GROUP,
        )
        .unwrap();
        assert_eq!(e.run_as.uid, 0);
        assert_eq!(e.cwd, Path::new("/work"));
        assert!(e.fallback_cwd);
        assert!(e.env.contains(&format!("PATH={DEFAULT_PATH}")));
        assert!(e.env.contains(&"TERM=vt100".into()));
        let e = resolve_with_databases(
            &s,
            &image,
            true,
            Path::new("/work"),
            None,
            false,
            PASSWD,
            GROUP,
        )
        .unwrap();
        assert!(!e.env.iter().any(|s| s.starts_with("TERM=")));
        let e = resolve_with_databases(
            &s,
            &image,
            true,
            Path::new("/work"),
            None,
            true,
            PASSWD,
            GROUP,
        )
        .unwrap();
        assert!(e.env.contains(&"TERM=dumb".into()));
        let s = Settings {
            working_dir: Some("/explicit".into()),
            ..s
        };
        let e = resolve_with_databases(
            &s,
            &image,
            true,
            Path::new("/work"),
            None,
            false,
            PASSWD,
            GROUP,
        )
        .unwrap();
        assert!(!e.fallback_cwd);
    }
}
