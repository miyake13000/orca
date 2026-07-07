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

|                     | setuid              | sudo                      |
| ------------------- | ------------------- | ------------------------- |
| Environment variables | preserved         | reset (`sudo -E` to preserve) |
| `orca run` user     | the invoking user   | root (`orca run --user $(id -u) --group $(id -g)` to keep) |
| Data location       | `$HOME/.local/share/orca` | `/root/.local/share/orca` |

## Command Reference

### Global Options

```
orca [--env <name-or-uuid>] <command> [args]
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
| `orca init <name> --image ubuntu:24.04` | Use a Docker image as base |
| `orca init <name> --keep` | Create without switching current environment |
| `orca use <name-or-uuid>` | Switch the current environment |
| `orca ls` | List all environments |
| `orca rm <name-or-uuid>` | Delete an environment entirely (asks for confirmation; `--yes` to skip) |
| `orca clean` | Discard uncommitted changes (destroy upper layer) |

### Running

| Command | Description |
|---|---|
| `orca run` | Enter the container (resumes from last state if uncommitted changes exist) |
| `orca run <cmd> [args]` | Run a specific command inside the container |
| `orca run --no-pid` / `--no-uts` / `--no-ipc` | Disable PID / UTS / IPC namespace isolation |
| `orca run --network` | Isolate the network namespace (shared with host by default) |
| `orca run --user <uid\|name>` | Run as this user inside the container |
| `orca run --group <gid\|name>` | Run with this group inside the container |

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
| `orca apply --dry-run` | Show changes without applying |
| `orca apply --yes` | Skip confirmation prompt |

> Only available for host-based containers. Docker image-based containers will error.

### Image Management

| Command | Description |
|---|---|
| `orca image pull ubuntu:24.04` | Pull a Docker image |
| `orca image ls` | List cached images |
| `orca image rm ubuntu:24.04` | Remove a cached image |

---

## Limitations

- Linux only
- Requires root privileges (sudo or setuid)
- Maximum 500 committed layers per container (OverlayFS limit)
- Filesystems mounted separately on the host (e.g. a separate `/home` partition) are not visible inside host-based containers
- `orca merge` is not yet implemented; use `orca rebase <newbase> <target>` instead (rebase target onto newbase)
- Concurrent access to the same container is not supported

## License

MIT
