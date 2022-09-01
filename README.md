# orca

## Summary
orca is lightweight container management tool

orca creates container from
1. Container Image in DockerHub (not require root)
2. host's root filesystem (require root)

## Prerequisities
### Debian or Arch Linux
Execute below command to be able to separate user_namespace with non-root user.  
1. `$ sudo sysctl -w kernel.unprivileged_userns_clone=1`

## Install orca
Download orca from [release page](https://github.com/miyake13000/orca/releases/latest).
Or execute below command to download command-line.
1. `$ curl -L https://github.com/miyake13000/orca/releases/latest/download/orca > orca`
2. `$ chmod +x ./orca`

### Optional
We recommend to install uidmap package.  
If uidmap is not installed, you cannot create new user in container.  
1. `$ sudo apt install uidmap`

## How to use
1. Use Container Image
   ```bash
   $ ./orca -d hello-world -t latest /hello # create container from hello-world:latest image
   ```
2. Use Host Image  
You want to chack if the compilation of vim works correctry, but you want not to leave garbage files created with the compilation.
In this case, orca just fits this need.
    ```bash
    $ sudo ./orca -H su - miyake # create container and switch user to miyake
    $ git clone https://github.com/vim/vim.git # you are already in container
    $ ls 
    vim other_files
    $ cd vim && make install
    $ exit # leave container
    $ ls
    other_files
    ```

## Uninstall
1. `rm ./orca`
2. `rm -rf $HOME/.local/share/orca`

## Build from source
### normal build
1. Install [Rust](https://www.rust-lang.org/tools/install)
2. Build orca
    ```bash
    $ cargo build --release
    ```
3. orca is placed 'target/release/orca'

### staticaly linked build
1. Install [docker](https://docs.docker.com/engine/install/)
2. Build orca
   ```bash
    $ ./static_build.sh
    ```
3. orca is placed 'target/x86_64-unknown-linux-musl/release/orca'

