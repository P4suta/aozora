# Release runbook

リリースは [release-plz](https://release-plz.dev/) が前段 (Cargo.toml の version bump、CHANGELOG.md 更新、PR 作成、annotated tag push) を担い、`.github/workflows/release.yml` が後段 (3 OS バイナリ + `aozora-ffi` C ABI のビルド、git-cliff によるリリースノート生成、GitHub Release 公開) を担う。

## 通常運用

1. `feat:` / `fix:` / `perf:` などの conventional commit を `main` に merge する
2. `.github/workflows/release-plz.yml` が `release-plz/main` ブランチに Release PR を自動で開く / 更新する
3. Release PR の差分を確認:
   - `Cargo.toml` の `[workspace.package].version` と内部 path dep の version 行が新バージョンに揃っている
   - `Cargo.lock` は resolver 再実行のみ
   - `CHANGELOG.md` は `cliff.toml` のグループ順 (Added / Fixed / Performance / Changed / Documentation / Tests / CI / Build / Chore / Reverted) で前 tag からの commit を反映している
4. Release PR を merge すると `release-plz` が `vX.Y.Z` annotated tag を push する
5. `release.yml` が tag を捕捉して 3 OS バイナリ + C ABI + SHA256SUMS + `git-cliff --latest` のノートで GitHub Release を公開する

## ローカル先読み

PR をマージする前に、release-plz が次に開く Release PR の内容を確認できる:

```bash
cargo binstall release-plz
release-plz update --dry-run --config release-plz.toml
```

各 crate の bump 結果と CHANGELOG diff、`cargo semver-checks` の警告が表示される。ファイルは書き換わらない。

## GitHub App セットアップ (1 回限り)

GITHUB_TOKEN で push された ref は他 workflow を起動しないという GitHub の仕様により、release-plz が打った tag で `release.yml` を発火させるには GitHub App installation token が必要。リポジトリ admin が以下を 1 回だけ実施する。

1. GitHub → Settings (個人) → Developer settings → GitHub Apps → New GitHub App
   - Name: `aozora-release-plz` (グローバル一意。衝突したら `-p4suta` を付ける)
   - Homepage URL: `https://github.com/P4suta/aozora`
   - Webhook: 無効化 (Active のチェックを外す。callback URL 不要)
   - Repository permissions:
     - **Contents**: Read & write (commit + tag push)
     - **Pull requests**: Read & write (Release PR 作成 / 更新)
   - Subscribe to events: 何も付けない
   - Where can this App be installed: Only on this account
2. App 設定ページ → Private keys → Generate a private key で `.pem` をダウンロード
3. App ページ → Install App → `P4suta/aozora` を選んでインストール
4. リポジトリ secrets を登録:
   ```bash
   gh secret set APP_ID --repo P4suta/aozora                                       # 数値 App ID
   gh secret set APP_PRIVATE_KEY --repo P4suta/aozora < <path-to-private-key>.pem  # PEM 全文
   ```
5. 動作確認: `gh workflow run release-plz.yml --repo P4suta/aozora`
   - `release-plz-pr` ジョブが緑になり、リリース対象がある場合は `release-plz/main` ブランチに PR が開く

## トラブルシュート

- **tag が push されたのに `release.yml` が走らない** — App token の `Contents: Read & write` 権限と、リポジトリへのインストール状態を確認
- **Release PR が出ない** — release-plz workflow run のログで「次の version は何か」「対象 commit が認識されているか」を確認。conventional commits の prefix (`feat:` / `fix:` / `perf:` / `refactor:` 等) が無いと bump 対象として拾われない
- **CHANGELOG が空** — `cliff.toml` の `commit_parsers` で全 commit が skip 対象になっていないかを確認
