# orca
## Install orca
### Linux (Debian or Ubuntu)
1. `$ sudo apt install git curl uidmap`
2. `$ curl -sLJO https://github.com/miyake13000/orca/releases/download/0.1/orca`
3. `$ chmod +x ./orca`

## Restriction
- Debian ディストリビューションで利用する場合，以下のコマンドを実行  
`$ sudo sysctl -w kernel.unprivileged_userns_clone=1`

## How to use
1. `$ ./orca`
2. `# apt install -y sl`
3. `# sl`
4. `# exit` 
