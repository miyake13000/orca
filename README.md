# orca
## Install orca
### Linux
1. `$ sudo apt install -y git curl newgidmap newuidmap`
2. `$ git clone https://github.com/miyake13000/orca`
3. `$ export PATH=$PATH:$(pwd)/orca`

## Restriction
- Debian ディストリビューションで利用する場合，以下のコマンドを実行  
`$ sudo sysctl -w kernel.unprivileged_userns_clone=1`

## How to use
1. `$ orca`
2. `# apt install -y sl`
3. `# sl`
4. `# exit` 
