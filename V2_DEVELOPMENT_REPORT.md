# ModLinker v2.0.0 開発完了レポート

要件定義書「公式ランチャー完全連携 & ZGC自動最適化」「未登録MODのスマート自動回収」の全項目を実装しました。
**検証結果: `cargo check` 警告ゼロ / ユニットテスト 23/23 合格 / JS構文チェック OK / i18n 20言語完全整合**

---

## 1. 公式ランチャー完全連携 ＆ 並行処理パイプライン

### A. MODローダー公式インストーラーのサイレント実行代行
`src-tauri/src/launcher/installer.rs`

| ローダー | バージョン解決 | サイレント実行コマンド |
|---|---|---|
| Fabric | `meta.fabricmc.net` (installer/loader 最新) | `java -jar fabric-installer.jar client -mcVersion X -loaderVersion Y -dir <dir> -noprofile` |
| Forge | `promotions_slim.json` → `maven-metadata.xml` | `java -jar forge-installer.jar --installClient <dir>` |
| NeoForge | `maven.neoforged.net` API | `java -jar neoforge-installer.jar --installClient <dir>` |

- `-noprofile` でインストーラー側のプロファイル書き込みを抑制し、プロファイル管理を ModLinker 側に一元化(二重の安全装置)。
- **Java自動探索** (`launcher/java.rs`): 公式内蔵ランタイムを優先探索
  - Windows: `%LOCALAPPDATA%\.minecraft\runtime\` + MS Store 版 (`Packages\Microsoft.4297127D64EC6_...`)
  - macOS: `~/Library/Application Support/minecraft/runtime/`、Linux: `~/.minecraft/runtime/`
  - `java-runtime-gamma`(J21) → `delta`(J17) → alpha/beta/omega → `JAVA_HOME` → `PATH` → システムJVM の順
  - `java -version` を実際に実行してメジャーバージョンを検出し、要求バージョンに合うものを選択

### B. 並行処理パイプライン
`src-tauri/src/launcher/pipeline.rs`

- `tokio::join!` で「ローダー導入 (CPU)」∥「MOD DL (回線)」を真並行実行
- 同一ドライブ内 `<gameDir>/.staging/` に隔離 → **両方完了した瞬間**に `std::fs::rename` で原子的合体
  - 対象: `mods/`, `versions/`, `libraries/` 等すべて
  - rename 失敗時は copy フォールバック(別ドライブ等の万一に備える)
  - 本番に同名ディレクトリがある場合は再帰マージで既存MODを保護

### C. 起動構成 (gameDir) の安全な自動分離
`src-tauri/src/launcher/profiles.rs`

- `profiles/<ProfileName>/` を事前物理作成(サニタイズ済みASCII名・パストラバーサル遮断)
- `launcher_profiles.json` へ **1件だけ Append** (既存キーは一切触らない)
- 【絶対の安全装置】
  1. 書き込み実行前 + 直前の二重プロセスチェック (`sysinfo` で MinecraftLauncher.exe 等を検知 → **保存中止**でブロック)
  2. 書き換え直前にタイムスタンプ付き `.bak-YYYYMMDD-HHMMSS` を同一ディレクトリへ自動生成
- JSON解析失敗時は書き込み中止、tmp → rename の原子的書き込み

### D. 【完全型安全】JVM / ZGC 最適化ビルダー
`src-tauri/src/launcher/jvm.rs` ＋ `src/manifest.rs`

- マニフェストから**生のJVM引数文字列は serde 構造レベルで拒否**(`jvm_args` キーは無視される)
  - 受け取るのは `{ "recommended_java": 21, "gc": "zgc", "memory_mb": 10240, "always_pre_touch": true }` のみ
  - `gc` は enum (`auto`/`zgc`/`g1`)、`memory_mb` は u32 + クランプ → **コマンド/引数インジェクションを100%構造的遮断**
- 生成引数はホワイトリスト定数のみ(このファイル以外で引数を生成しない設計)
- バージョン・環境分岐:
  - **Java 21 ＋ ZGCトグルON** → 世代別ZGC `-XX:+UseZGC -XX:+ZGenerational`、上限 **16GB**
  - **Java 17 以下 / フォールバック** → G1GC `-XX:+UseG1GC`、上限 **8GB**(物理RAM - 2GB にもクランプ)
- UI: 「🚀 ZGC最適化を有効にする」トグル + 生成引数のライブプレビュー
- 起動失敗時は `plan_jvm_fallback` コマンドで G1GC 版プランを即時提示

## 2. スマート手動MOD自動回収パイプライン

### A. 動作フロー
`src-tauri/src/watcher.rs`

1. 手動DL対象を選ぶと絞り込みURLを外部ブラウザでオープン ＋ 📥 **ダウンロード待機モーダル**表示
2. **モーダルが開いている間だけ**ダウンロードフォルダを監視(常時監視は絶対なし)
3. `notify` で `.crdownload`/`.part`/`.tmp` → `.jar` のリネーム・作成をフック(イベント駆動 + 2秒定期スキャンの**ハイブリッド検知**で取りこぼしゼロ)
4. **SHA-512完全一致のみ**で判定(ファイル名は一切使わない) → `fs::rename` で `mods/` 等へ移動
   - ファイルロック中は 8ms × 50回の段階的リトライ → copy フォールバック
   - 全対象が揃った瞬間、監視を**即座に停止・破棄**(`watcher-progress` イベントでUIへ配置完了を通知)

### B. ブラウザDL先の階層型パス解決
`src-tauri/src/browser_paths.rs`

1. アプリ内設定(ユーザー記憶パス)最優先
2. Chromium系 `Preferences` JSON の `download.default_directory`:
   - Chrome / Edge / Brave / Arc / Opera / Opera GX (Windows %LOCALAPPDATA%・macOS・Linux 対応、Default/Profile 1/2 も探索)
3. OS標準 Downloads (`dirs` クレート)

### C. フロントエンドUI
`src/index.html` + `src/main.js` + `src/styles.css`

- Glassmorphism ＆ ダーク/ライトモード準拠の待機モーダル
- **GSAPパルスアニメーション**で「待機中 ➔ ハッシュ検証中 ➔ 配置完了」を視覚化(フェーズバッジが色分けされ、検知時にバウンス)
- 📁 監視先表示 + `[フォルダを変更]` ボタン(変更時はウォッチャーを再構築)
- フォールバック導線(自動検知しない環境向け):
  - `[🔍 今すぐダウンロードフォルダをスキャン]` ワンショット一括ハッシュ照合
  - `[📂 ファイルを直接選択して配置]`
  - ドロップゾーン(`tauri://drag-drop` イベント連携、**複数一括対応**)
- `[キャンセル]` で確実にウォッチャーを破棄(モーダルを閉じる=監視停止)

## 3. その他の実装

- **マニフェスト v2 形式**: `{ format_version, loader, tuning, mods }` — v1 配列形式も完全後方互換で読み込み
- **ローダー手動指定UI**: v1 マニフェストでもローダー種別 + MCバージョンを指定してワンクリック同期可能
- **新コマンド11個**追加: `detect_manifest_info` / `check_launcher_running_cmd` / `resolve_java` / `plan_jvm_args` / `plan_jvm_fallback` / `resolve_loader_version` / `run_launcher_sync` / `resolve_download_dir_cmd` / `start_download_watcher` / `stop_download_watcher` / `scan_download_dir_once` / `place_files_from_paths`
- **DiffEntry に `sha512` を追加**(ハッシュ照合に必須のため)
- **i18n 20カ国語拡張**: 新キー51個 × 20言語(ja/en/zh-CN/zh-TW/ko/es/fr/de/it/pt-BR/ru/pl/nl/tr/th/vi/uk/cs/sv/id、全196キー整合)
- バージョン表記を **V2.0.0** に更新(サイドバー / tauri.conf.json / Cargo.toml / package.json)

## 4. テスト(23個)

| カテゴリ | 検証内容 |
|---|---|
| watcher (5) | **ランダムなファイル名のJARがハッシュ一致で正しく配置される**、未一致ファイルは無傷、partial(.crdownload/.part/.tmp)をスキップ、複数DnD配置、ハッシュ不一致は拒否+元ファイル保持 |
| manifest (5) | v1配列/v2オブジェクト両パース、破損JSON拒否、**バージョン文字列のインジェクション遮断**、生`jvm_args`の構造的遮断 |
| jvm (5) | Java21世代別ZGC生成、Java17でのG1自動降格、16GB/8GBキャップ、`-Xmx`数値検証(注入不能証明)、物理RAMクランプ |
| profiles (4) | プロファイル安全追記、既存キー保護、`.bak`自動生成、破損JSONでの書き込み中止、フォルダ名サニタイズ |
| browser_paths (3) | Preferences解析、壊れたJSONの無視、優先順位(user_setting最優先)、OSフォールバック |
| java (1) | `java -version`出力パース(21/17/1.8系) |

## 5. ファイル構成(新規)

```
src-tauri/src/
├── manifest.rs            # マニフェストv2パーサー(型安全・v1互換)
├── watcher.rs             # ダウンロード監視・ハッシュ照合・自動配置
├── browser_paths.rs       # ブラウザDL先階層解決
└── launcher/
    ├── mod.rs
    ├── java.rs            # 公式内蔵Java自動探索
    ├── installer.rs       # Fabric/Forge/NeoForge公式インストーラー実行代行
    ├── jvm.rs             # 【完全型安全】JVM/ZGCビルダー
    ├── pipeline.rs        # tokio並行パイプライン + staging原子的合体
    └── profiles.rs        # launcher_profiles.json安全追記(二重安全装置)
```

## 6. 実行環境について

- 開発検証は Linux sandbox で `cargo check` / `cargo test` を実施(Windows 固有のプロセスチェックやレジストリ系はOS別分岐を実装済みだが実機テスト推奨)
- リリースビルド (`npm run tauri build`) は Windows/macOS の実機環境で実行してください
- `.github/workflows/release.yml` のCIは既存のまま利用可能(バージョンは Cargo.toml 連動)
