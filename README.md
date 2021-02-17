# orca

## Summury
Lightweight rootless container management tool

## Prerequisities
### Debian
Execute below command to be able to separate user_namespace by non-root user  
`$ sudo sysctl -w kernel.unprivileged_userns_clone=1`

## Install orca
### Linux
#### Debian or Ubuntu
1. `$ sudo apt install curl uidmap`
2. `$ curl -L https://github.com/miyake13000/orca/releases/download/0.1/orca > orca`
3. `$ chmod +x ./orca`

## How to use
1. `$ ./orca -d hello-world -t latest /hello` 

## Uninstall
1. `$ rm ./orca`
2. `$ rm -rf $HOME/.local/orca`  

## Build static linked binary
You must build static linked openssl before build static linked binary.  
Execute below command once.
1. `$ sudo apt install musl-tools gcc openssl`
2. `$ sudo bash build_static_openssl.sh`

Execute below command to build static linked binary.
1. `$ bash create_static_binary.sh`