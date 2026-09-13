# orca

English | [日本語](README_ja.md)

A Git-like sandboxed version control system

## Motivation

* Try `make install` without worrying about polluting your system.
* Debug with destructive file changes, knowing you can always roll back.

orca achieves this with containers based on your current environment, plus version control over the whole container.

```bash
# Create an environment (env)
orca init myenv

# Enter a container based on your current environment and work freely
orca run bash
# $ make install    <- changes are isolated & the host is untouched
# $ exit            <- leaving the container brings you back to the original environment

# Commit the changes like Git, if you want to keep them
orca commit -m "install foo and its dependencies"

# Apply the container state to the host (the files from make install get placed on the host)
orca apply

# OR discard the changes
orca clean
```

## Features

- **OverlayFS isolation** — your base image (host rootfs or Docker image) is never modified
- **Git-style version control** — commit, branch, checkout, reset, rebase, and log your environment changes
- **Docker image support** — pull and use any Docker image as a base
- **Host rootfs support** — use your live system as a base for zero-setup containers
- **Branching** — experiment in parallel, merge (rebase) or discard branches freely

## Installation

1. Download orca

```bash
wget https://github.com/miyake13000/orca/releases/latest/download/orca
```

2. Give orca root privileges (sudo or setuid)
    - setuid (recommended): `sudo chown root:root orca && sudo chmod 4755 orca`
    - sudo: `echo "alias orca='sudo \$(which orca)'" >> ~/.bashrc && source ~/.bashrc`

Both installation methods use the same configured execution identity (host default: root).
Host environment variables are not inherited except TERM for interactive runs.
The default data location follows the invoking process's HOME/XDG configuration;
use `--root` to choose it explicitly under sudo. See Configuration below.

## Command Reference

### Global Options

```
orca [--root <path>] [--env|-e <name-or-uuid>] <command> [args]
```

The target environment is resolved in this order:

1. `--env` option
2. `ORCA_ENV` environment variable
3. Current environment (set via `orca use`)

---

### Environment Management

| Command | Description |
|---|---|
| `orca init <name>` | Create a new environment from host rootfs (default) |
| `orca init <name> --host` | Explicitly use host rootfs as base |
| `orca init <name> --image\|-i ubuntu:24.04` | Use a Docker image as base |
| `orca init <name> --keep\|-k` | Create without switching current environment |
| `orca use <name-or-uuid>` | Switch the current environment |
| `orca ls` | List all environments |
| `orca rm <name-or-uuid>` | Delete an environment entirely (asks for confirmation; `--yes|-y` to skip) |
| `orca clean` | Discard uncommitted changes (destroy upper layer) |

### Running

| Command | Description |
|---|---|
| `orca run` | Enter the container (resumes from last state if uncommitted changes exist) |
| `orca run <cmd> [args]` | Run a specific command inside the container |
| `orca run --no-pid` / `--no-uts` / `--no-ipc` | Disable PID / UTS / IPC namespace isolation |
| `orca run --network\|-n` | Isolate the network namespace (shared with host by default) |
| `orca run --user\|-u <uid\|name>` | Run as this user inside the container |
| `orca run --group\|-g <gid\|name>` | Run with this group inside the container |

### Version Control

| Command | Description |
|---|---|
| `orca commit -m <message>` | Commit current changes (not allowed in detached HEAD) |
| `orca diff` | Show uncommitted file changes |
| `orca diff <A> [<B>]` | Show changes of commit A (against the base, or against commit B) |
| `orca log` | Show commit history |
| `orca branch <n>` | Create a new branch from current HEAD |
| `orca branch -d <n>` | Delete a branch |
| `orca branch -a` | List all branches |
| `orca checkout <branch-or-commit>` | Switch to a branch or commit (requires clean state) |
| `orca checkout -b <n>` | Create a new branch and switch to it |
| `orca reset <commit>` | Hard reset to a previous commit (not allowed in detached HEAD) |
| `orca rebase <newbase> <target>` | Rebase target branch onto newbase |
| `orca merge <branch>` | Merge a branch (not yet implemented) |
| `orca gc` | Remove unreachable commits and objects |

> `ROOT` (the initial commit) can be used as the target of `checkout` / `reset`

### Apply

| Command | Description |
|---|---|
| `orca apply` | Apply all committed changes + uncommitted changes to host |
| `orca apply --no-upper` | Apply committed changes only (exclude uncommitted changes) |
| `orca apply --dry-run\|-n` | Show changes without applying |
| `orca apply --yes\|-y` | Skip confirmation prompt |

> Only available for host-based containers. Docker image-based containers will error.

### Image Management

| Command | Description |
|---|---|
| `orca image pull ubuntu:24.04` | Pull a Docker image |
| `orca image ls` | List cached images |
| `orca image rm ubuntu:24.04` | Remove a cached image |

---

## Configuration (config.toml)

User settings live in `<root>/config.toml`. Environment metadata in
`<root>/envs/envs.toml` is managed by orca and is not a user configuration file.
There is no migration from the old settings format.

The data root is selected by `--root`, then `ORCA_ROOT`, then the platform's
local data directory (`~/.local/share/orca` normally). To use your own data
through sudo, use `sudo orca --root "$HOME/.local/share/orca" run`.
A relative `--root` is resolved against the invoking working directory.

All settings are optional. Missing config means no overrides; invalid TOML,
unknown fields and invalid setting values are errors. Environment names match
exactly, including case; quote names containing dots. Names may be configured
before creation, and configuration survives deletion/recreation of an env.

Priority: **arguments > named environment > global > image/tool defaults**.

```toml
[global]
blacklist = ["*.log", "*.tmp"]

[global.env]
EDITOR = "vim"
LANG = "ja_JP.UTF-8"

[global.namespaces]
pid = true
uts = true
ipc = true
network = false

[envs."dev"]
cmd = ["/bin/bash", "-l"]
working_dir = "/workspace"
user = "1000"
group = "1000"
blacklist = ["!important.log"]

[envs."dev".env]
EDITOR = "nvim"

[envs."dev".namespaces]
network = true
```

| Setting | CLI equivalent | Merge rule |
|---|---|---|
| `cmd` | `run <command> [args]` | Replace complete argv; explicit empty argv is invalid |
| `env` | `run --set-env KEY=VALUE` (repeatable) | Merge by key, later values win |
| `working_dir` | `run --working-dir /path` | Replace; absolute container path only |
| `user` / `group` | `run --user` / `--group` | Replace |
| `namespaces.pid/uts/ipc/network` | `run --pid=true`, `--uts=false`, etc. | Replace; true isolates, false shares |
| `blacklist` | `diff/apply --blacklist PATTERN` (repeatable) | Append rules, last matching rule wins |

`cmd` replaces the entire image `Entrypoint + Cmd`; orca has no separate
`entrypoint` setting. Values are literal: no `$VAR`, `${VAR}`, or `~` expansion.
An empty environment value is allowed; removing a variable is not supported.

Users and groups are resolved **after pivot_root**, using only the container's
`/etc/passwd` and `/etc/group` (not NSS). Numeric IDs need not be registered;
unknown names abort startup. The image's `USER` is the default, or UID 0 if
absent (also the host default), irrespective of sudo/setuid invocation.
An explicit user replaces the image's complete `USER` declaration; a separately
configured group overrides the primary group. Without a group override, use
the passwd primary GID (or 0 for an unregistered UID) and supplementary group
memberships. An explicit group clears supplementary groups.

Host environment variables are **not inherited**, except TERM during PTY runs.
Environment construction order is: fixed PATH → image environment → resolved
user's HOME/USER/LOGNAME/SHELL → terminal TERM → global/env/argument variables.
The default PATH is `/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`.
For an unregistered UID, HOME is `/`, SHELL is `/bin/sh`, and USER/LOGNAME are
the numeric UID. TERM comes from the caller during PTY runs, or `dumb` if absent;
explicit settings override it. Non-PTY runs do not synthesize TERM.
The selected shell may subsequently process its own startup files.

Without `cmd`, host environments use the resolved user's shell; external images
use `Entrypoint + Cmd`. Without `working_dir`, host environments use the invoking
cwd, external images use `WorkingDir` (or `/`). Only the host's implicit cwd may
fall back to `/`; a failed fallback, explicit cwd failure or image cwd failure
aborts startup. Cwd is entered after switching credentials.

Blacklist patterns support root-anchored paths (`/var/log` or patterns containing
`/`), names at any depth, `*` (not across `/`), and `?`. `!pattern` cancels an
exclusion; an excluded parent directory must itself be re-included before any
child can be re-included. This is a subset of Git ignore syntax. Rules accumulate
in global → env → argument order; `[]` adds nothing, it does not reset earlier
rules. Blacklists affect **diff and apply only**; commits still retain all files.

## Limitations

- Linux only
- Requires root privileges (sudo or setuid)
- Maximum 500 committed layers per container (OverlayFS limit)
- Filesystems mounted separately on the host (e.g. a separate `/home` partition) are not visible inside host-based containers
- `orca merge` is not yet implemented; use `orca rebase <newbase> <target>` instead (rebase target onto newbase)
- Concurrent access to the same container is not supported

## License

MIT
