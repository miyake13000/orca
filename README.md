# orca

English | [日本語](README_ja.md)

A container environment manager with Git-like version control, built in Rust.

orca lets you spin up isolated Linux containers, make changes freely, and commit or discard them — just like version-controlling your environment.

## Motivation

Ever wanted to try `make install` without worrying about your system? Or debug a production issue by making destructive changes to your environment, knowing you can instantly roll back?

orca solves this by combining **OverlayFS-based container isolation** with **Git-style version control** for your filesystem changes.

```bash
# Create a container based on your host system
orca init myenv

# Enter the container and do whatever you want
orca run bash
# $ make install    <- safe! changes are isolated
# $ apt install ... <- go wild
# $ exit

# Commit the changes, just like Git
orca commit -m "install foo and its dependencies"

# Or throw them away
orca clean
```

## Features

- **OverlayFS isolation** — your base image (host rootfs or Docker image) is never modified
- **Git-style version control** — commit, branch, checkout, reset, rebase, and log your environment changes
- **Docker image support** — pull and use any Docker image as a base
- **Host rootfs support** — use your live system as a base for zero-setup containers
- **Branching** — experiment in parallel, merge or discard branches freely

## Requirements

- Linux kernel with OverlayFS support
- Root privileges (or `CAP_SYS_ADMIN`)

## Installation

```bash
cargo install orca
```

## Quick Start

```bash
# Create a container from your host system (default)
orca init myenv

# Or from a Docker image
orca init myenv --image ubuntu:24.04

# Enter the container
orca run

# Make changes, then commit
orca commit -m "installed build dependencies"

# View history
orca log

# Create a branch to try something risky
orca branch experiment
orca checkout experiment
orca run bash
orca commit -m "tried the risky thing"

# Rebase back (or just discard the branch)
orca checkout main
orca rebase main experiment
```

## CLI Reference

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

> `ROOT` can be used as the target of `checkout` / `reset` — it resolves to the initial (parentless) commit.

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

- Linux only (OverlayFS is a Linux kernel feature)
- Requires root or `CAP_SYS_ADMIN`
- Maximum 500 committed layers per container (OverlayFS kernel limit)
- Filesystems mounted separately on the host (e.g. a separate `/home` partition) are not visible inside host-based containers (OverlayFS lower layers do not cross mount points)
- `orca merge` is not yet implemented; use `orca rebase <newbase> <target>` instead (rebase target onto newbase)
- Concurrent access to the same container is not supported

## License

MIT
