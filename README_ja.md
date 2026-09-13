# orca

[English](README.md) | 日本語

Git ライクなサンドボックスバージョンコントロールシステム

## モチベーション

* システムを汚す心配をせずに `make install` を試したい．
* ロールバックできると分かった上で，ファイル変更を伴うデバッグをしたい．

このような要件を，現在の環境をベースとしたコンテナ + コンテナ全体のバージョン管理で実現

```bash
# 環境 (env) を作成
orca init myenv

# 現在の環境ベースのコンテナに入って，作業を行う
orca run bash
# $ make install    <- 変更は隔離 & ホストには影響なし
# $ exit            <- コンテナを抜けると，元の環境に復帰

# 必要なら Git のように変更をコミット
orca commit -m "install foo and its dependencies"

# コンテナ環境をホストにも適用 (make install のファイルがホストにも配置)
orca apply

# OR 必要ないなら変更を破棄
orca clean
```

## 特徴

- **OverlayFS による隔離** — ベースイメージ (ホスト rootfs や Docker イメージ) は決して変更されない
- **Git スタイルのバージョン管理** — 環境の変更を commit / branch / checkout / reset / rebase / log できる
- **Docker イメージ対応** — 任意の Docker イメージを pull してベースとして使える
- **ホスト rootfs 対応** — 稼働中のシステムをそのままベースにでき，セットアップ不要でコンテナを作れる
- **ブランチ** — 並行して実験し，ブランチを自由にマージ (rebase) したり破棄したりできる


## インストール

1. orca のダウンロード

```bash
wget https://github.com/miyake13000/orca/releases/latest/download/orca
```

2. orca に root 権限 (sudo または setuid) を付与
    - setuid の場合(推奨): `sudo chown root:root orca && sudo chmod 4755 orca`
    - sudo の場合: `echo "alias orca='sudo \$(which orca)'" >> ~/.bashrc && source ~/.bashrc`

どちらの起動方法も設定で決定したユーザを使う (ホストの既定値は root)。
ホストの環境変数は対話時の TERM 以外は継承しない。
既定のデータ保存先は起動プロセスの HOME/XDG 設定に従うため、sudo では
`--root` で明示指定できる。詳細は「設定」を参照。

## コマンド仕様

### グローバルオプション

```
orca [--root <path>] [--env|-e <name-or-uuid>] <command> [args]
```

対象の環境は次の優先順位で決まる:

1. `--env` オプション
2. 環境変数 `ORCA_ENV`
3. カレント環境 (`orca use` で設定)

---

### 環境管理

| コマンド | 説明 |
|---|---|
| `orca init <name>` | ホスト rootfs から新しい環境を作成 (デフォルト) |
| `orca init <name> --host` | ホスト rootfs をベースにすることを明示 |
| `orca init <name> --image\|-i ubuntu:24.04` | Docker イメージをベースに作成 |
| `orca init <name> --keep\|-k` | カレント環境を切り替えずに作成 |
| `orca use <name-or-uuid>` | カレント環境を切り替え |
| `orca ls` | 環境の一覧を表示 |
| `orca rm <name-or-uuid>` | 環境を完全に削除 (確認プロンプトあり，`--yes|-y` でスキップ) |
| `orca clean` | 未コミットの変更を破棄 (upper レイヤを破壊) |

### 実行

| コマンド | 説明 |
|---|---|
| `orca run` | コンテナに入る (未コミットの変更があれば前回の状態から再開) |
| `orca run <cmd> [args]` | コンテナ内で指定したコマンドを実行 |
| `orca run --no-pid` / `--no-uts` / `--no-ipc` | PID / UTS / IPC namespace の分離を無効化 |
| `orca run --network\|-n` | network namespace を分離 (デフォルトはホストと共有) |
| `orca run --user\|-u <uid\|name>` | コンテナ内の実行ユーザを指定 |
| `orca run --group\|-g <gid\|name>` | コンテナ内の実行グループを指定 |

### バージョン管理

| コマンド | 説明 |
|---|---|
| `orca commit -m <message>` | 現在の変更をコミット (detached HEAD では不可) |
| `orca diff` | 未コミットのファイル変更を表示 |
| `orca diff <A> [<B>]` | コミット A の変更を表示 (基底基準．B 指定時は B 基準) |
| `orca log` | コミット履歴を表示 |
| `orca branch <n>` | 現在の HEAD からブランチを作成 |
| `orca branch -d <n>` | ブランチを削除 |
| `orca branch -a` | ブランチの一覧を表示 |
| `orca checkout <branch-or-commit>` | ブランチまたはコミットへ移動 (クリーンな状態が必要) |
| `orca checkout -b <n>` | ブランチを作成して移動 |
| `orca reset <commit>` | 過去のコミットへ hard reset (detached HEAD では不可) |
| `orca rebase <newbase> <target>` | target ブランチを newbase に rebase |
| `orca merge <branch>` | ブランチをマージ (未実装) |
| `orca gc` | 到達不能なコミットとオブジェクトを削除 |

> `checkout` / `reset` の対象には `ROOT` (初期コミット，`initial commit`) を指定できる

### apply

| コマンド | 説明 |
|---|---|
| `orca apply` | コミット済み + 未コミットの全変更をホストに適用 |
| `orca apply --no-upper` | コミット済みの変更のみ適用 (未コミット分を除外) |
| `orca apply --dry-run\|-n` | 適用せずに変更内容だけ表示 |
| `orca apply --yes\|-y` | 確認プロンプトをスキップ |

> ホストベースのコンテナ専用．Docker イメージベースのコンテナではエラーになる．

### イメージ管理

| コマンド | 説明 |
|---|---|
| `orca image pull ubuntu:24.04` | Docker イメージを pull |
| `orca image ls` | キャッシュ済みイメージの一覧を表示 |
| `orca image rm ubuntu:24.04` | キャッシュ済みイメージを削除 |

---

blacklist が効くのは `orca diff` の表示と `orca apply` の適用対象のみ．commit には影響せず，レイヤには全ファイルが記録される．
## 設定 (config.toml)

ユーザ設定は `<root>/config.toml` に記述する。`<root>/envs/envs.toml` は
orca が管理する環境一覧・current 等の情報で、ユーザ設定を含まない。
旧設定からの移行処理は設けない。

データルートは `--root` > `ORCA_ROOT` > 標準のローカルデータディレクトリ
(通常 `~/.local/share/orca`) で決まる。
`sudo orca --root "$HOME/.local/share/orca" run` でユーザ側のデータを利用できる。
相対 `--root` は起動元の CWD を基準に絶対パスへ変換する。

全項目は省略可能。設定ファイルがなければ設定なし。不正な TOML・未知の項目・
不正な値はエラー。env 名は大文字・小文字を区別した完全一致で、未作成の名前も
記述できる。削除後も設定は残り、同名で再作成すると再び適用される。

優先順位は **引数 > env 固有 > global > イメージ・ツールの既定値**。

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

| 設定 | 対応する引数 | 合成規則 |
|---|---|---|
| `cmd` | `run <command> [args]` | argv 全体を置換。明示的な空配列はエラー |
| `env` | `run --set-env KEY=VALUE` (複数指定可) | キーごとに上書き |
| `working_dir` | `run --working-dir /path` | 置換。コンテナ内の絶対パスのみ |
| `user` / `group` | `run --user` / `--group` | 置換 |
| `namespaces.pid/uts/ipc/network` | `run --pid=true`、`--uts=false` 等 | 置換。true=分離、false=共有 |
| `blacklist` | `diff/apply --blacklist PATTERN` (複数指定可) | 順に連結し、最後に一致した規則を採用 |

orca の `entrypoint` 設定は廃止し、`cmd` でイメージの `Entrypoint + Cmd` 全体を
置き換える。値はリテラルで `$VAR`・`${VAR}`・`~` を展開しない。
環境変数の空文字は許可するが、変数を削除する指定は設けない。

user/group は **pivot_root 後**の `/etc/passwd`・`/etc/group` で解決する
(NSS は使わない)。数値 ID は未登録でも受け入れ、名前が見つからなければ起動を中止する。
既定ユーザはイメージの `USER`、未宣言時およびホストベースでは UID 0。
sudo / setuid による自動選択はしない。user の明示指定はイメージの `USER` 宣言全体を
置き換え、group の明示指定があれば優先する。group 未指定時は passwd のプライマリ GID
(未登録 UID は 0) と所属補助グループを使い、group 明示時は補助グループを空にする。

ホストの環境変数は **PTY 実行時の TERM 以外は継承しない**。
固定 PATH → イメージ ENV → ユーザ由来 HOME/USER/LOGNAME/SHELL → 端末 TERM →
global/env/引数 ENV の順に設定する。固定 PATH は
`/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`。
未登録 UID は HOME=`/`、SHELL=`/bin/sh`、USER/LOGNAME=UID の文字列表現とする。
PTY 時の TERM は起動元の値、なければ `dumb`。明示 ENV が優先する。
非 PTY 時は TERM を自動補完しない。起動後のシェルが rc 等で環境を変更することはある。

cmd 未指定時、ホストは解決したユーザのシェル、外部イメージは `Entrypoint + Cmd`。
CWD 未指定時、ホストは起動元 CWD、外部イメージは `WorkingDir` (未宣言なら `/`)。
ホストの既定 CWD だけは移動失敗時に `/` へフォールバックし、それも失敗すれば停止する。
明示 CWD と外部イメージの CWD での失敗は即時停止する。移動は権限変更後に行う。

blacklist はルート起点のパス (`/var/log` や `/` を含むもの)、任意階層の名前、
`*` (区切りを跨がない)、`?` を扱う Git ignore 風のサブセット。
`!pattern` で除外を解除できるが、親ディレクトリが除外されている間は子だけを再包含できない。
global → env → 引数の順に規則を連結する。`[]` は追加なしで、継承規則のリセットではない。
適用対象は **diff / apply のみ**。commit は全ファイルを保持する。

## 制限事項

- Linux 専用
- root 権限が必要 (sudo または setuid)
- コンテナあたりのコミット済みレイヤは最大 500 (OverlayFS の制限)
- ホスト上で別マウントされているファイルシステム (別パーティションの `/home` など) は，ホストベースのコンテナからは見えない
- `orca merge` は未実装．代わりに `orca rebase <newbase> <target>` を使う (target を newbase に rebase)
- 同一コンテナへの並行アクセスは未サポート

## ライセンス

MIT
