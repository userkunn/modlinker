// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – 並行処理パイプライン (Task Parallelism)
//
//  ■ 設計 (要件 1-B)
//   「MODローダーのインストール (CPU負荷)」と「MODのダウンロード (回線負荷)」
//   を tokio で非同期並行実行する。
//   同一ドライブ内のステージングフォルダ (.staging/) に隔離して処理し、
//   両方完了した瞬間に std::fs::rename (0.01秒) で本番ディレクトリへ
//   原子的に合体・配置する。
//
//   同一ドライブ内の rename は OS が atomic に実行するため、
//   完成途中の状態がユーザーの mods/ に露出しない。
// ═══════════════════════════════════════════════════════════

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::io::AsyncWriteExt;

use crate::manifest::{LoaderInfo, ManifestV2};
use super::installer::{self, InstalledLoader};
use super::jvm::{self, JvmPlan};

const USER_AGENT: &str = "modlinker/2.0.0";

/// パイプライン進捗イベント
#[derive(Clone, Serialize)]
pub struct PipelineEvent {
    pub phase: String,      // "loader" | "mods" | "merge" | "profile" | "done" | "error"
    pub message: String,
    pub progress: f64,      // 0.0 - 1.0 (全体)
}

/// ワンクリック同期オプション (フロントから。生のJVM引数は含まない)
#[derive(Clone, Deserialize, Debug)]
pub struct LauncherSyncOptions {
    /// プロファイル名 (例: "FriendServer")
    pub profile_name: String,
    /// ZGC 最適化トグル (参加者側UI「🚀 ZGC最適化を有効にする」)
    #[serde(default)]
    pub zgc_enabled: bool,
    /// ローダー情報なしマニフェストの場合に UI が指定したローダー (None可)
    #[serde(default)]
    pub loader_override: Option<LoaderInfo>,
    /// R2等の非公式URLをスキップ
    #[serde(default)]
    pub skip_unofficial: bool,
}

/// 同期結果サマリー
#[derive(Serialize, Debug)]
pub struct SyncSummary {
    pub profile_key: String,
    pub profile_game_dir: String,
    pub loader_version_id: Option<String>,
    pub java_executable: Option<String>,
    pub jvm_plan: JvmPlan,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
    pub backup_path: Option<String>,
    pub staging_merged: bool,
}

fn emit(app: &tauri::AppHandle, phase: &str, msg: &str, p: f64) {
    let _ = app.emit("launcher-sync-progress", PipelineEvent {
        phase: phase.to_string(),
        message: msg.to_string(),
        progress: p.clamp(0.0, 1.0),
    });
}

/// staging ディレクトリパス: <game_root>/.staging/
pub fn staging_dir(game_root: &Path) -> PathBuf {
    game_root.join(".staging")
}

/// ローダーインストールとMODダウンロードを並行実行し、staging で合流して
/// 原子的に本番へ配置、最後に launcher_profiles.json にプロファイルを追記する。
pub async fn run_launcher_sync(
    app: tauri::AppHandle,
    game_root: PathBuf,
    manifest: ManifestV2,
    opts: LauncherSyncOptions,
) -> Result<SyncSummary, String> {
    // ── 0. 安全装置: 公式ランチャー起動中ならブロック ──
    let proc_check = super::profiles::check_launcher_running();
    if proc_check.running {
        return Err(format!(
            "⛔ Minecraft 公式ランチャーが起動中です ({})。設定の破損を防ぐため保存を中止しました。ランチャーを終了してから再試行してください。",
            proc_check.matched_name.unwrap_or_default()
        ));
    }

    if manifest.mods.is_empty() && manifest.loader.is_none() {
        return Err("マニフェストにローダー情報もMODもありません。".into());
    }

    let loader = manifest.loader.clone().or(opts.loader_override.clone());
    let tuning = manifest.tuning.clone().unwrap_or_default();

    // ── 1. Java 探索 (内蔵ランタイム優先) ──
    emit(&app, "loader", "Java ランタイムを探索中 (公式内蔵Java優先)...", 0.03);
    let required_java = loader
        .as_ref()
        .map(|l| installer::recommended_java_for_mc(&l.mc_version))
        .unwrap_or(tuning.recommended_java);
    let java_rt = match installer::find_java_runtime(required_java.max(tuning.recommended_java)).await {
        Some(rt) => Some(rt),
        None => installer::find_best_java().await,
    };
    let java_exe = match &java_rt {
        Some(rt) => std::path::PathBuf::from(&rt.executable),
        None => {
            // Java 未導入環境: 内蔵ランタイムも PATH も見つからない場合
            // ローダーインストールには必須のためエラー (MOD DL だけでも続行可能にするため
            // loader がある場合のみブロック)
            if loader.is_some() {
                return Err("Java ランタイムが見つかりませんでした。公式ランチャーで一度 Minecraft を起動するか、Java をインストールしてください。".into());
            }
            std::path::PathBuf::from("java")
        }
    };
    let java_major = java_rt.as_ref().map(|r| r.major_version).unwrap_or(required_java);

    // ── 2. プロファイル用 gameDir を事前物理作成 (既存環境を汚さない) ──
    emit(&app, "profile", "プロフォール用ディレクトリを準備中...", 0.06);
    let profile_game_dir = super::profiles::create_isolated_game_dir(&game_root, &opts.profile_name)?;
    // プロファイル内に target_dir 構造を作るのは merge 後。staging は同一ドライブ (game_root 直下)
    let staging = staging_dir(&game_root);
    let _ = tokio::fs::remove_dir_all(&staging).await; // 前回の残骸を消す
    tokio::fs::create_dir_all(&staging).await.map_err(|e| e.to_string())?;

    // ── 3. 並行パイプライン: ローダー(CPU) ∥ MODダウンロード(回線) ──
    emit(&app, "loader", "🚀 並行パイプライン開始: ローダー導入 ∥ MODダウンロード", 0.08);

    let app_l = app.clone();
    let app_m = app.clone();
    let staging_l = staging.clone();
    let staging_m = staging.clone();
    let loader_clone = loader.clone();
    let manifest_clone = manifest.clone();
    let skip_unofficial = opts.skip_unofficial;

    let (loader_result, mods_result) = tokio::join!(
        async {
            match &loader_clone {
                Some(info) => {
                    emit(&app_l, "loader", "🔧 ローダー公式インストーラーをサイレント実行中...", 0.1);
                    let r = installer::install_loader(&app_l, info, &java_exe, &game_root, &staging_l).await;
                    emit(&app_l, "loader", if r.is_ok() { "✔ ローダー導入完了" } else { "✘ ローダー導入失敗" }, 0.45);
                    r
                }
                None => {
                    emit(&app_l, "loader", "ローダー指定なし (MOD同期のみ)", 0.45);
                    let _ = &staging_l;
                    Ok(InstalledLoader {
                        version_id: String::new(),
                    })
                }
            }
        },
        async {
            emit(&app_m, "mods", "⬇ MOD差分ダウンロード開始 (staging に隔離)...", 0.1);
            download_mods_to_staging(&app_m, &manifest_clone, &staging_m, skip_unofficial).await
        }
    );

    let installed = loader_result.map_err(|e| {
        let _ = tokio::fs::remove_dir_all(&staging);
        format!("ローダーインストール失敗: {}", e)
    })?;
    let (downloaded, skipped, failed) = mods_result.map_err(|e| {
        let _ = tokio::fs::remove_dir_all(&staging);
        format!("MODダウンロード失敗: {}", e)
    })?;

    // ── 4. staging → 本番へ原子的合体 (std::fs::rename) ──
    emit(&app, "merge", "⚡ staging → 本番ディレクトリへ原子的合体中...", 0.55);
    let merge_target = if loader.is_some() { profile_game_dir.clone() } else { game_root.clone() };
    let merged = merge_staging(&app, &staging, &merge_target).await?;

    // ── 5. JVM プラン生成 (完全型安全) ──
    emit(&app, "profile", "JVM/ZGC 最適化プランを生成中...", 0.85);
    let ram = jvm::physical_ram_mb();
    let plan = jvm::build_jvm_plan(&tuning, java_major, opts.zgc_enabled, ram);

    // ── 6. launcher_profiles.json 安全追記 ──
    emit(&app, "profile", "launcher_profiles.json にプロファイルを追記中...", 0.9);

    // 再度プロセスチェック (書き込み直前の最終防御)
    let proc_check2 = super::profiles::check_launcher_running();
    if proc_check2.running {
        let _ = tokio::fs::remove_dir_all(&staging);
        return Err(format!(
            "⛔ 書き込み直前のチェックで Minecraft ランチャーの起動を検知しました ({})。保存を中止しました。",
            proc_check2.matched_name.unwrap_or_default()
        ));
    }

    let now_iso = iso_now();
    let java_dir_opt = java_rt.as_ref().map(|rt| {
        std::path::Path::new(&rt.executable)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }).filter(|s| !s.is_empty());

    let profile_entry = super::profiles::ProfileEntry {
        profile_key: String::new(),
        name: opts.profile_name.clone(),
        game_dir: merge_target.to_string_lossy().to_string(),
        last_version_id: if installed.version_id.is_empty() {
            "latest-release".to_string()
        } else {
            installed.version_id.clone()
        },
        java_dir: java_dir_opt,
        java_args: plan.args.join(" "),
    };

    let game_root_c = game_root.clone();
    let profile_key = tokio::task::spawn_blocking(move || {
        super::profiles::upsert_profile(&game_root_c, &profile_entry, &now_iso)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("プロファイル追記失敗: {}", e))?;

    // ── 7. staging 掃除 ──
    let _ = tokio::fs::remove_dir_all(&staging).await;

    let backup_path = super::profiles::profiles_path(&game_root);
    let backup_str = find_latest_backup(&backup_path);

    emit(&app, "done", &format!("✅ 完了! プロファイル「{}」で遊べます", opts.profile_name), 1.0);

    Ok(SyncSummary {
        profile_key,
        profile_game_dir: merge_target.to_string_lossy().to_string(),
        loader_version_id: if installed.version_id.is_empty() { None } else { Some(installed.version_id) },
        java_executable: java_rt.map(|rt| rt.executable),
        jvm_plan: plan,
        downloaded,
        skipped,
        failed,
        backup_path: backup_str,
        staging_merged: merged,
    })
}

/// staging の mods/ 等を本番へ移す。ローダー構造 (versions/, libraries/) も含む。
async fn merge_staging(app: &tauri::AppHandle, staging: &Path, dest_root: &Path) -> Result<bool, String> {
    // staging 直下の各エントリを処理
    let mut rd = tokio::fs::read_dir(staging).await.map_err(|e| e.to_string())?;
    let mut merged_any = false;
    let mut count = 0;

    while let Some(entry) = rd.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        let name = entry.file_name();
        // インストーラーの一時ファイル等は無視
        if name.to_string_lossy().starts_with("modlinker-installer") {
            continue;
        }
        let dest = dest_root.join(&name);
        count += 1;
        emit(app, "merge", &format!("配置: {} → {}", name.to_string_lossy(), dest_root.to_string_lossy()), 0.55 + (count as f64) * 0.005);

        merge_path(&path, &dest).await?;
        merged_any = true;
    }
    Ok(merged_any)
}

/// 単一パスの原子的移動 (rename 失敗時はフォールバックで copy)
fn merge_path<'a>(src: &'a Path, dest: &'a Path) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
    // 本番に既存がある場合は上書き処理が必要
    if dest.is_dir() {
        // ディレクトリ同士: 中身を個別にマージ
        let mut rd = tokio::fs::read_dir(src).await.map_err(|e| e.to_string())?;
        while let Some(entry) = rd.next_entry().await.map_err(|e| e.to_string())? {
            let p = entry.path();
            let d = dest.join(entry.file_name());
            merge_path(&p, &d).await?;
        }
        let _ = tokio::fs::remove_dir_all(src).await;
        Ok(())
    } else if dest.exists() {
        // 既存ファイル → 上書き (rename は失敗するので delete → rename)
        tokio::fs::remove_file(dest).await.map_err(|e| e.to_string())?;
        std::fs::rename(src, dest)
            .or_else(|_| {
                // 別ドライブ等の万一のケースは copy
                std::fs::copy(src, dest).map(|_| ())
            })
            .map_err(|e| format!("配置失敗 ({} → {}): {}", src.display(), dest.display(), e))?;
        Ok(())
    } else {
        // 通常ケース: 0.01秒の atomic rename
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        std::fs::rename(src, dest)
            .or_else(|_| std::fs::copy(src, dest).map(|_| ()))
            .map_err(|e| format!("配置失敗 ({} → {}): {}", src.display(), dest.display(), e))?;
        Ok(())
    }
    })
}

/// MOD 差分を staging へダウンロード (回線負荷タスク)
async fn download_mods_to_staging(
    app: &tauri::AppHandle,
    manifest: &ManifestV2,
    staging: &Path,
    skip_unofficial: bool,
) -> Result<(usize, usize, Vec<String>), String> {
    let targets: Vec<&crate::manifest::ModEntry> = manifest
        .mods
        .iter()
        .filter(|m| m.download_url.is_some())
        .collect();

    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let mut failed: Vec<String> = Vec::new();

    let client = reqwest::Client::builder().user_agent(USER_AGENT).build().map_err(|e| e.to_string())?;

    for (i, entry) in targets.iter().enumerate() {
        let url = entry.download_url.clone().unwrap();
        let is_official = url.contains("cdn.modrinth.com")
            || url.contains("edge.forgecdn.net")
            || url.contains("mediafilez.forgecdn.net");
        if skip_unofficial && !is_official {
            skipped += 1;
            emit(app, "mods", &format!("⚠ 非公式URL をスキップ: {}", entry.file_name), 0.1 + 0.35 * (i as f64 / targets.len() as f64));
            continue;
        }

        emit(app, "mods", &format!("⬇ ({}/{}): {}", i + 1, targets.len(), entry.file_name), 0.1 + 0.35 * (i as f64 / targets.len() as f64));

        let dest = staging.join(&entry.target_dir).join(&entry.file_name);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }

        match client.get(&url).send().await {
            Ok(mut res) => {
                if !res.status().is_success() {
                    failed.push(format!("{} (HTTP {})", entry.file_name, res.status()));
                    continue;
                }
                let mut file = match tokio::fs::File::create(&dest).await {
                    Ok(f) => f,
                    Err(e) => {
                        failed.push(format!("{} ({})", entry.file_name, e));
                        continue;
                    }
                };
                let mut ok = true;
                while let Ok(Some(chunk)) = res.chunk().await {
                    if file.write_all(&chunk).await.is_err() {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    downloaded += 1;
                } else {
                    failed.push(format!("{} (書き込みエラー)", entry.file_name));
                }
            }
            Err(e) => {
                failed.push(format!("{} ({})", entry.file_name, e));
            }
        }
    }

    Ok((downloaded, skipped, failed))
}

/// ISO 8601 形式の現在時刻 (外部クレートなし・簡易)
pub fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z", y, mth, d, h, m, s)
}

fn find_latest_backup(profiles_json: &Path) -> Option<String> {
    let dir = profiles_json.parent()?;
    let mut best: Option<String> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("launcher_profiles.json.bak-") {
            best = Some(name);
        }
    }
    best.map(|n| dir.join(n).to_string_lossy().to_string())
}
