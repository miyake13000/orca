# orca

Sandbox version control system

## Summary

Orca creates sandboxes from

1. host's filesystem
2. container (DockerHub)  
   and version control both host and container environment (by versioning entire filesystem with OverlayFS)

Read [mechanism](docs/mechanism.md) for more information.

## Install orca

Download orca from [release page](https://github.com/miyake13000/orca/releases/latest).
Or execute below command to download command-line.

```bash
$ wget https://github.com/miyake13000/orca/releases/latest/download/orca
$ chmod +x ./orca
```

### Optional

Orca needs root priviledge, so make orca available with sudo.

```bash
$ sudo mv ./orca /usr/bin/
```

Or, setuid to orca

```bash
$ sudo chmod 6755 ./orca
```

## How to use

1. Initialize (once)
   ```bash
   $ orca init # Use host envrionment
   ```
   Or, you can use container image
   ```bash
   $ orca init --image ubuntu:22.04 --name ubuntu-test
   ```
2. Run orca
   ```bash
   # current files: example.c
   $ orca run # Enter sandbox
   $ apt update && apt install -y clang
   $ clang -o example example.c
   # current files: example example.c
   $ ./example
   $ exit # Exit from sandbox
   # current files: example.c
   ```
   Or, you can use container image created orca init
   ```bash
   $ orca --name ubuntu-test run
   $ apt update && apt install -y clang
   $ clang -o example example.c
   ```
3. Commit environment
   ```bash
   $ orca commit --message "Install clang"
   $ orca log
   commit: 8dfb0a6c3c943d14ab4cf745d1c761cc6f386219
     date: 2024-05-03 10:46:11.868560348 +09:00
     message: Install clang
   ```
4. Create branch

   ```bash
   $ orca branch libc
   $ orca checkout libc
   $ orca run bash -c "apt update && apt install -y 2.35-0ubuntu3"
   $ orca run gcc -o example example.com
   $ orca log
   commit: 408dfdc46bb489eafbf6e38acbeae7656d0c31ec
     date: 2024-05-03 10:49:25.145348653 +09:00
     message: Downgrade libc

   commit: 8dfb0a6c3c943d14ab4cf745d1c761cc6f386219
     date: 2024-05-03 10:46:11.868560348 +09:00
     message: Install clang
   ```

5. Merge branch

   ```bash
   $ orca checkout main
   $ orca merge libc
   $ orca log
   commit: 408dfdc46bb489eafbf6e38acbeae7656d0c31ec
     date: 2024-05-03 10:49:25.145348653 +09:00
     message: Downgrade libc

   commit: 8dfb0a6c3c943d14ab4cf745d1c761cc6f386219
     date: 2024-05-03 10:46:11.868560348 +09:00
     message: Install clang
   ```

6. Reset commit
   ```bash
   $ orca reset 8dfb0a
   $ orca log
   commit: 8dfb0a6c3c943d14ab4cf745d1c761cc6f386219
     date: 2024-05-03 10:46:11.868560348 +09:00
     message: Install clang
   ```

## Uninstall

1. `sudo rm $(which orca)`
2. `sudo rm -rf $HOME/.local/share/orca`

## Build from source

### normal build

1. Install [Rust](https://www.rust-lang.org/tools/install)
2. Build orca
   ```bash
   $ cargo build --release
   ```
3. Orca is placed 'target/release/orca'

### staticaly linked build

1. Install 'x86_64-unknown-linux-musl' target
   ```bash
   $ rustup target add x86_64-unknown-linux-musl
   ```
2. Build orca
   ```bash
   $ cargo build --release --target x86_64-unknown-linux-musl
   ```
3. Orca is placed 'target/x86_64-unknown-linux-musl/release/orca'
