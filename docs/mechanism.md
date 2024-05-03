## orca 仕組み
`orca init`を実行すると `$HOME/.orca`が作成される．
以下にディレクトリ構成を説明する．

### ディレクトリ構成
```
.orca
├── host
│   ├── commits
│   ├── layers
│   │   ├── 01234678910abcdefg
│   │   ├── 1234567891011abcde
│   │   └── ...
│   ├── upper
│   ├── rootfs
│   ├── tmp
│   │   ├── fake_rootfs
│   │   ├── fake_upper
│   │   ├── fake_work
│   │   └── ...
│   └── work
├── container_A
│   ├── commits
│   ├── image
│   ├── layers
│   │   └── ...
│   ├── upper
│   ├── rootfs
│   ├── tmp
│   └── work
└── ...
```

* host / container_A  
    ホスト環境の場合は host を，コンテナ環境の場合はユーザが指定した名前のディレクトリが作成される．
    それぞれのディレクトリ構造は以下の内容で統一されている．
    * commits  
        コミット情報を保存するファイル．詳細は後述する．
    * layers  
        各コミットごとの変更ファイル (差分) が配置されている．
        ディレクトリ名は対応するコミットIDと同じである．
        このディレクトリ群の中から一部，または全部を順序付けて lowerdir として指定する．
    * upper  
        コミット前の変更ファイルが配置される．
        OverlayFSのupperdirとして指定されるディレクトリ．
    * rootfs  
        OverlayFSがマウントされるディレクトリ．
        orca起動時はこのディレクトリはルートファイルシステムと同じディレクトリ構成を持つ．
        orca使用時の環境は，このディレクトリにchroot (正確には pivot_root) した環境である．
    * tmp  
        コンテナ作成における一時ファイルを保存したり，OverlayFSを仮マウントするため(ホストのみ，後述)に利用されるディレクトリ．
    * work  
        OverlayFSのworkdirとして指定されるディレクトリ．
    * image  
        コンテナイメージの場合のみ存在．
        コンテナイメージ本体のファイルシステムが配置されている．
        OverlayFSのマウントの際，lowerdirに指定される．

### OverlayFSのマウント設定
本来，以下の設定でマウントすることで，ホストのコンテナ化が達成できる．
* upperdir = .orca/upper
* lowerdir = /, .orca/layers/012..., .orca/layers/123...
* mount_point = .orca/rootfs

しかし，上記の設定ではマウントに失敗する．
これは，lowerdirの各ディレクトリは包含関係 (overlapping) にあってはならないためである．
この問題に対し，orcaは以下のようにOverlayFSを2回マウントすることで対処している．

* 仮マウント (1回目のマウント)
    * upperdir = .orca/tmp/fake_upper
    * lowerdir = /
    * mount_point = .orca/tmp/fake_rootfs
* マウント (2回目のマウント)
    * upperdir = .orca/upper
    * lowerdir = .orca/tmp/rootfs, .orca/layers/012..., .orca/layers/123...
    * mount_point = .orca/rootfs

上記の仮マウントを行うことで，2回目のマウントで lowerdir が overlapping することを防いでいる．
なお，仮マウントに用いるファイルシステムはOverlayFSでなくとも良く，ルートファイルシステムを透過的に見せられるファイルシステムであれば何でも良い．
しかし，バインドマウントは無効であるため，最も使用が容易な透過的ファイルシステムとして OverlayFSを用いている．

### コミットファイルの書式
.orca/commits にコミット情報がTOML形式で記述されており，以下の書式を取る．
```toml
[[commits]]
id = "012345678910abcdefg"
date = "2024-05-03 10:46:11.868560348 +09:00"
message = "Crate a"

[[commits]]
id = "1234567891011abcde"
parent_id = "012345678910abcdefg"
date = "2024-05-03 11:28:44.935689194 +09:00"
message = "Crate b"

[head]
branch_name = "main"
commit_id = "1234567891011abcde"
detached = false

[[branches]]
name = "main"
commit_id = "1234567891011abcde"

[[branches]]
name = "sub"
commit_id = "012345678910abcdefg"
```
* commits  
    コミット情報の配列．
    以下の情報を持つ．
    * id  
        コミットID．
        現状は chrono::offset::Local::now() で生成した現在時刻のインスタンスを Sha-1 にかけることで生成している．
    * parent_id  
        親コミットのID．オプショナル．
        存在しない場合，それが最初のコミットとみなされる．
    * date  
        コミットを行った際の時刻．
    * message  
        コミットメッセージ．オプショナル．
        ユーザがログからコミットの情報を判別するために付けられる情報．
* head  
    orcaがどのコミットを対象として環境を作成するかを示すための情報．
    以下の要素を持つ．
    * branch_name  
        特定のブランチを見ている場合，そのブランチ名がここに入る．
        以下の detached が true の場合，無視される．
    * commit_id  
        特定のコミットを見ている場合，そのコミットIDがここに入る．
        以下の detached が false の場合，無視される．
    * detached  
        bool型．
        今見ているコミットが，ブランチに紐付いている(false)か，そうでない(true)かを示す．
* branches  
    ブランチ情報の配列．
    以下の要素を持つ．
    * name  
        ブランチ名．
    * commit_id  
        そのブランチがどのコミットに紐付いているかを示す．
