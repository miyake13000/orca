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
orca apply HEAD

# OR 必要ないなら変更を破棄
orca clean
```

## 特徴

- **OverlayFS による隔離** — ベースイメージ (ホスト rootfs や Docker イメージ) は決して変更されない
- **Git スタイルのバージョン管理** — 環境の変更を commit / branch / checkout / reset / rebase / log できる
- **Docker イメージ対応** — 任意の Docker イメージを pull してベースとして使える
- **ホスト rootfs 対応** — 稼働中のシステムをそのままベースにでき，セットアップ不要でコンテナを作れる
- **ブランチ** — 並行して実験し，ブランチを自由にマージ (rebase) したり破棄したりできる

## 動作要件

- OverlayFS をサポートする Linux カーネル
- root 権限 (または `CAP_SYS_ADMIN`)

## インストール

```bash
cargo install orca
```

## クイックスタート

```bash
# ホストシステムからコンテナを作成 (デフォルト)
orca init myenv

# あるいは Docker イメージから
orca init myenv --image ubuntu:24.04

# コンテナに入る
orca run

# 変更を加えてコミット
orca commit -m "installed build dependencies"

# 履歴を見る
orca log

# 危険な実験用にブランチを切る
orca branch experiment
orca checkout experiment
orca run bash
orca commit -m "tried the risky thing"

# rebase で取り込む (あるいはブランチごと捨てる)
orca checkout main
orca rebase main experiment
```

## CLI リファレンス

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

> `checkout` / `reset` の対象には `ROOT` を指定できる — parent を持たない初期コミットに解決される．

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

## 制限事項

- Linux 専用 (OverlayFS は Linux カーネルの機能)
- root または `CAP_SYS_ADMIN` が必要
- コンテナあたりのコミット済みレイヤは最大 500 (OverlayFS のカーネル制限)
- ホスト上で別マウントされているファイルシステム (別パーティションの `/home` など) は，ホストベースのコンテナからは見えない (OverlayFS の lower レイヤはマウントポイントを跨がないため)
- `orca merge` は未実装．代わりに `orca rebase <newbase> <target>` を使う (target を newbase に rebase)
- 同一コンテナへの並行アクセスは非サポート

## ライセンス

MIT
