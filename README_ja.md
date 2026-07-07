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

|                    | setuid              | sudo                      |
| ------------------ | ------------------- | ------------------------- |
| 環境変数            | 維持                 | リセット (`sudo -E` で維持) |
| `orca run` のユーザ | 現在のユーザ          | root (`orca run --user $(id -u) --group $(id -g)`で維持) |
| データ保存場所       | `$HOME/.local/share/orca` | `/root/.local/share/orca` |


## コマンド仕様

### グローバルオプション

```
orca [--env <name-or-uuid>] <command> [args]
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
| `orca init <name> --image ubuntu:24.04` | Docker イメージをベースに作成 |
| `orca init <name> --keep` | カレント環境を切り替えずに作成 |
| `orca use <name-or-uuid>` | カレント環境を切り替え |
| `orca ls` | 環境の一覧を表示 |
| `orca rm <name-or-uuid>` | 環境を完全に削除 (確認プロンプトあり，`--yes` でスキップ) |
| `orca clean` | 未コミットの変更を破棄 (upper レイヤを破壊) |

### 実行

| コマンド | 説明 |
|---|---|
| `orca run` | コンテナに入る (未コミットの変更があれば前回の状態から再開) |
| `orca run <cmd> [args]` | コンテナ内で指定したコマンドを実行 |
| `orca run --no-pid` / `--no-uts` / `--no-ipc` | PID / UTS / IPC namespace の分離を無効化 |
| `orca run --network` | network namespace を分離 (デフォルトはホストと共有) |
| `orca run --user <uid\|name>` | コンテナ内の実行ユーザを指定 |
| `orca run --group <gid\|name>` | コンテナ内の実行グループを指定 |

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
| `orca apply --dry-run` | 適用せずに変更内容だけ表示 |
| `orca apply --yes` | 確認プロンプトをスキップ |

> ホストベースのコンテナ専用．Docker イメージベースのコンテナではエラーになる．

### イメージ管理

| コマンド | 説明 |
|---|---|
| `orca image pull ubuntu:24.04` | Docker イメージを pull |
| `orca image ls` | キャッシュ済みイメージの一覧を表示 |
| `orca image rm ubuntu:24.04` | キャッシュ済みイメージを削除 |

---

blacklist が効くのは `orca diff` の表示と `orca apply` の適用対象のみ．commit には影響せず，レイヤには全ファイルが記録される．
## 設定 (envs.toml)

データ保存場所の `envs/envs.toml` を編集すると，全 env 共通 (`[defaults]`) と env 個別 (`[envs.settings]`) の設定を書ける．優先度は **コマンドライン引数 > env 個別 > 共通 > イメージの宣言** (blacklist のみ共通と個別の和集合)．

```toml
[defaults]                  # 全 env 共通
env = ["EDITOR=vim"]
blacklist = ["*_history", ".cache"]

[[envs]]
# ...orca が管理するフィールド...

[envs.settings]             # この env だけの設定 (全フィールド任意)
cmd = ["/bin/zsh"]
working_dir = "/root"
blacklist = ["/var/log"]
```

| フィールド | 説明 |
|---|---|
| `entrypoint` / `cmd` | 起動コマンドの上書き |
| `env` | 環境変数の追加/上書き (`KEY=VALUE`) |
| `working_dir` | 作業ディレクトリの上書き |
| `blacklist` | diff / apply から除外するパス (下記) |

blacklist は Git ignore 風のパターンで指定する:

- `/xxx/yyy` — ルート起点のパス指定 (スラッシュを含むパターンはアンカー扱い)
- `xxx` — ファイル/ディレクトリ名一致 (任意の深さ)
- ワイルドカード `*` (パス区切りは跨がない)・`?`
- ディレクトリにマッチすると配下も丸ごと除外

## 制限事項

- Linux 専用
- root 権限が必要 (sudo または setuid)
- コンテナあたりのコミット済みレイヤは最大 500 (OverlayFS の制限)
- ホスト上で別マウントされているファイルシステム (別パーティションの `/home` など) は，ホストベースのコンテナからは見えない
- `orca merge` は未実装．代わりに `orca rebase <newbase> <target>` を使う (target を newbase に rebase)
- 同一コンテナへの並行アクセスは未サポート

## ライセンス

MIT
