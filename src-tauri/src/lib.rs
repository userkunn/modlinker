use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use aws_config::Region;
use aws_sdk_s3::{Client, config::Credentials, config::Builder as S3ConfigBuilder};
use aws_sdk_s3::primitives::ByteStream;
use uuid::Uuid;

// ── v2.0.0 モジュール ────────────────────────────────────
mod browser_paths;
mod launcher;
mod manifest;
mod watcher;

use launcher::jvm::{self, JvmPlan};
use launcher::pipeline::LauncherSyncOptions;
use manifest::{LoaderInfo, ManifestV2};
use watcher::WatchTarget;

// ── 型定義 ──────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct ProgressPayload {
    stage: String,
    message: String,
    progress: f64,
}

/// マニフェストの各エントリ
#[derive(Clone, Serialize, Deserialize)]
struct ModEntry {
    file_name: String,
    target_dir: String,
    sha512: String,
    download_url: Option<String>,
    project_name: Option<String>,
    version_number: Option<String>,
    source: String,   // "modrinth", "curseforge", "manual", "unknown"
    page_url: Option<String>, // 手動DL用のブラウザリンク
    matched: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct DiffEntry {
    file_name: String,
    target_dir: String,
    action: String,
    url: Option<String>,
    page_url: Option<String>,
    source: String,
    #[serde(default)]
    sha512: Option<String>, // v2.0: 手動MOD自動回収のハッシュ照合に使用
}

#[derive(Clone, Serialize, Deserialize)]
struct DownloadTarget {
    file_name: String,
    target_dir: String,
    url: String,
}

// ── Modrinth API 型 ─────────────────────────────────────

#[derive(Serialize)]
struct ModrinthHashRequest {
    hashes: Vec<String>,
    algorithm: String,
}

#[derive(Deserialize, Debug)]
struct ModrinthVersion {
    name: Option<String>,
    version_number: Option<String>,
    files: Vec<ModrinthFile>,
}

#[derive(Deserialize, Debug)]
struct ModrinthFile {
    url: String,
    hashes: ModrinthFileHashes,
}

#[derive(Deserialize, Debug)]
struct ModrinthFileHashes {
    sha512: Option<String>,
}

// ── CurseForge API 型 ───────────────────────────────────

#[derive(Serialize)]
struct CfFingerprintRequest {
    fingerprints: Vec<u32>,
}

#[derive(Deserialize, Debug)]
struct CfFingerprintResponse {
    data: CfFingerprintData,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct CfFingerprintData {
    exact_matches: Vec<CfFingerprintMatch>,
}

#[derive(Deserialize, Debug)]
struct CfFingerprintMatch {
    file: CfFile,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct CfFile {
    display_name: Option<String>,
    download_url: Option<String>,
    file_fingerprint: u32,
    // mod ページ用
    mod_id: Option<u64>,
}

// ── CurseForge フィンガープリント (Murmur2 変種) ────────

fn cf_is_whitespace(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\r' | b' ')
}

fn compute_cf_fingerprint(data: &[u8]) -> u32 {
    let multiplex: u32 = 1540483477;
    let normalized_len = data.iter().filter(|&&b| !cf_is_whitespace(b)).count() as u32;

    let mut num2: u32 = 1u32 ^ normalized_len;
    let mut num3: u32 = 0;
    let mut num4: u32 = 0;

    for &b in data {
        if !cf_is_whitespace(b) {
            num3 |= (b as u32) << num4;
            num4 += 8;

            if num4 == 32 {
                let num6 = num3.wrapping_mul(multiplex);
                let num7 = (num6 ^ (num6 >> 24)).wrapping_mul(multiplex);
                num2 = num2.wrapping_mul(multiplex) ^ num7;
                num3 = 0;
                num4 = 0;
            }
        }
    }

    if num4 > 0 {
        num2 = (num2 ^ num3).wrapping_mul(multiplex);
    }

    let num6 = (num2 ^ (num2 >> 13)).wrapping_mul(multiplex);
    num6 ^ (num6 >> 15)
}

// ── ヘルパー関数 ─────────────────────────────────────────

fn extract_mod_name(path: &Path) -> Option<String> {
    use std::fs::File;
    use zip::ZipArchive;
    use std::io::Read;

    let file = File::open(path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;

    // Fabric
    if let Ok(mut fabric_file) = archive.by_name("fabric.mod.json") {
        let mut content = String::new();
        if fabric_file.read_to_string(&mut content).is_ok() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(name) = json.get("name").and_then(|v| v.as_str()) {
                    return Some(name.to_string());
                }
            }
        }
    }

    // Forge / NeoForge
    let toml_paths = ["META-INF/neoforge.mods.toml", "META-INF/mods.toml", "mods.toml", "neoforge.mods.toml"];
    for toml_path in toml_paths {
        if let Ok(mut toml_file) = archive.by_name(toml_path) {
            let mut content = String::new();
            if toml_file.read_to_string(&mut content).is_ok() {
                if let Ok(toml_val) = content.parse::<toml::Value>() {
                    if let Some(mods) = toml_val.get("mods").and_then(|v| v.as_array()) {
                        if let Some(first_mod) = mods.first() {
                            if let Some(name) = first_mod.get("displayName").and_then(|v| v.as_str()) {
                                return Some(name.to_string());
                            } else if let Some(mod_id) = first_mod.get("modId").and_then(|v| v.as_str()) {
                                return Some(mod_id.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Older Forge
    if let Ok(mut info_file) = archive.by_name("mcmod.info") {
        let mut content = String::new();
        if info_file.read_to_string(&mut content).is_ok() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(arr) = json.as_array() {
                    if let Some(first) = arr.first() {
                        if let Some(name) = first.get("name").and_then(|v| v.as_str()) {
                            return Some(name.to_string());
                        }
                    }
                } else if let Some(mod_list) = json.get("modList").and_then(|v| v.as_array()) {
                    if let Some(first) = mod_list.first() {
                        if let Some(name) = first.get("name").and_then(|v| v.as_str()) {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

async fn compute_sha512(path: &Path) -> Result<String, String> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("ファイル読み取りエラー: {}: {}", path.display(), e))?;
    let hash = Sha512::digest(&data);
    Ok(format!("{:x}", hash))
}

/// ファイルを読み取って SHA-512 と CF fingerprint を同時に返す
async fn compute_hashes(path: &Path) -> Result<(String, u32), String> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("ファイル読み取りエラー: {}: {}", path.display(), e))?;
    let sha512 = format!("{:x}", Sha512::digest(&data));
    let fingerprint = compute_cf_fingerprint(&data);
    Ok((sha512, fingerprint))
}

fn emit_progress(app: &AppHandle, stage: &str, message: &str, progress: f64) {
    let _ = app.emit(
        "sync-progress",
        ProgressPayload {
            stage: stage.to_string(),
            message: message.to_string(),
            progress,
        },
    );
}

// ── コマンド: マニフェスト生成 ─────────────────────────────

#[tauri::command]
async fn sync_mods(app: AppHandle, game_path: String, cf_api_key: Option<String>) -> Result<Vec<ModEntry>, String> {
    let game_dir = Path::new(&game_path);
    if !game_dir.exists() || !game_dir.is_dir() {
        return Err(format!("指定されたゲームディレクトリが無効です: {}", game_path));
    }

    emit_progress(&app, "scanning", "対象フォルダをスキャン中...", 0.05);

    let target_dirs = vec!["mods", "tacz", "shaderpacks"];
    let mut scan_targets: Vec<(PathBuf, String)> = Vec::new();

    for dir_name in target_dirs {
        let dir_path = game_dir.join(dir_name);
        if !dir_path.exists() { continue; }
        let files: Vec<_> = WalkDir::new(&dir_path)
            .min_depth(1).max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().map_or(false, |ext| {
                    ext.eq_ignore_ascii_case("jar") || ext.eq_ignore_ascii_case("zip")
                })
            })
            .map(|e| (e.into_path(), dir_name.to_string()))
            .collect();
        scan_targets.extend(files);
    }

    let total_files = scan_targets.len();
    if total_files == 0 {
        emit_progress(&app, "done", "対象ファイルが見つかりませんでした。", 1.0);
        return Ok(Vec::new());
    }

    emit_progress(&app, "scanning", &format!("{} 個のファイルを検出。", total_files), 0.1);
    emit_progress(&app, "hashing", "ハッシュを計算中...", 0.15);

    // SHA-512 と CF fingerprint を直列で計算して進捗を送信（並列によるメモリ不足とフリーズを防止）
    let mut file_hashes: Vec<(String, String, String, u32)> = Vec::new();
    let mut processed = 0;
    
    for (path, target_dir) in scan_targets {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        emit_progress(&app, "hashing", &format!("ハッシュを計算中... {} ({}/{})", file_name, processed + 1, total_files), 0.15 + (0.20 * (processed as f64 / total_files as f64)));
        
        match compute_hashes(&path).await {
            Ok((sha512, fp)) => {
                file_hashes.push((file_name, target_dir, sha512, fp));
            }
            Err(e) => {
                emit_progress(&app, "warning", &format!("⚠ {}", e), 0.15);
            }
        }
        processed += 1;
    }

    emit_progress(&app, "hashing", &format!("{}/{} ハッシュ完了。", file_hashes.len(), total_files), 0.35);

    if file_hashes.is_empty() {
        return Ok(Vec::new());
    }

    let client = reqwest::Client::builder()
        .user_agent("modlinker/0.3.0")
        .build()
        .map_err(|e| format!("HTTPクライアント初期化エラー: {}", e))?;

    // ═══ ステップ1: Modrinth API 照合 ═══
    emit_progress(&app, "querying", "Modrinth API に照合中...", 0.4);

    let hashes: Vec<String> = file_hashes.iter().map(|(_, _, h, _)| h.clone()).collect();
    let request_body = ModrinthHashRequest {
        hashes,
        algorithm: "sha512".to_string(),
    };

    let mut modrinth_result: HashMap<String, ModrinthVersion> = HashMap::new();
    match client.post("https://api.modrinth.com/v2/version_files")
        .json(&request_body).send().await
    {
        Ok(response) if response.status().is_success() => {
            modrinth_result = response.json().await.unwrap_or_default();
        }
        Ok(_) => {
            emit_progress(&app, "warning", "⚠ Modrinth API: 一部照合失敗", 0.5);
        }
        Err(e) => {
            emit_progress(&app, "warning", &format!("⚠ Modrinth API エラー: {}", e), 0.5);
        }
    }

    let modrinth_matched: usize = modrinth_result.len();
    emit_progress(&app, "querying", &format!("Modrinth: {} 件一致", modrinth_matched), 0.55);

    // ═══ ステップ2: CurseForge API 照合 (キーがある場合のみ) ═══
    let has_cf_key = cf_api_key.as_ref().map_or(false, |k| !k.trim().is_empty());
    let mut cf_result: HashMap<u32, CfFile> = HashMap::new();

    if has_cf_key {
        let cf_key = cf_api_key.as_ref().unwrap().trim().to_string();
        // Modrinth で見つからなかったファイルの fingerprint だけを抽出
        let unmatched_fps: Vec<u32> = file_hashes.iter()
            .filter(|(_, _, sha512, _)| !modrinth_result.contains_key(sha512))
            .map(|(_, _, _, fp)| *fp)
            .collect();

        if !unmatched_fps.is_empty() {
            emit_progress(&app, "querying", &format!("CurseForge API に {} 件照合中...", unmatched_fps.len()), 0.6);

            let cf_body = CfFingerprintRequest { fingerprints: unmatched_fps };
            match client.post("https://api.curseforge.com/v1/fingerprints")
                .header("x-api-key", &cf_key)
                .header("Accept", "application/json")
                .json(&cf_body)
                .send().await
            {
                Ok(response) if response.status().is_success() => {
                    if let Ok(cf_resp) = response.json::<CfFingerprintResponse>().await {
                        for m in cf_resp.data.exact_matches {
                            cf_result.insert(m.file.file_fingerprint, m.file);
                        }
                    }
                    emit_progress(&app, "querying", &format!("CurseForge: {} 件一致", cf_result.len()), 0.7);
                }
                Ok(resp) => {
                    let status = resp.status();
                    emit_progress(&app, "warning", &format!("⚠ CurseForge API: HTTP {} (キーを確認してください)", status), 0.7);
                }
                Err(e) => {
                    emit_progress(&app, "warning", &format!("⚠ CurseForge API エラー: {}", e), 0.7);
                }
            }
        }
    } else {
        emit_progress(&app, "querying", "CurseForge: APIキー未設定 (スキップ)", 0.6);
    }

    // ═══ マニフェスト構築 ═══
    emit_progress(&app, "building", "マニフェストを生成中...", 0.8);

    let mut manifest: Vec<ModEntry> = Vec::new();
    for (file_name, target_dir, sha512, fingerprint) in &file_hashes {
        // 1. Modrinth でヒット?
        if let Some(version) = modrinth_result.get(sha512) {
            let download_url = version.files.iter()
                .find(|f| f.hashes.sha512.as_deref() == Some(sha512.as_str()))
                .map(|f| f.url.clone())
                .or_else(|| version.files.first().map(|f| f.url.clone()));

            manifest.push(ModEntry {
                file_name: file_name.clone(),
                target_dir: target_dir.clone(),
                sha512: sha512.clone(),
                download_url,
                project_name: version.name.clone(),
                version_number: version.version_number.clone(),
                source: "modrinth".to_string(),
                page_url: None,
                matched: true,
            });
            continue;
        }

        // 2. CurseForge でヒット?
        if let Some(cf_file) = cf_result.get(fingerprint) {
            let page_url = cf_file.mod_id.map(|id| format!("https://www.curseforge.com/minecraft/mc-mods/{}", id));

            manifest.push(ModEntry {
                file_name: file_name.clone(),
                target_dir: target_dir.clone(),
                sha512: sha512.clone(),
                download_url: cf_file.download_url.clone(),
                project_name: cf_file.display_name.clone(),
                version_number: None,
                source: if cf_file.download_url.is_some() { "curseforge".to_string() } else { "manual".to_string() },
                page_url,
                matched: true,
            });
            continue;
        }

        // 3. どちらにも見つからない場合はGoogle検索へのリンクを付与
        let file_path_clone = game_dir.join(target_dir).join(file_name);
        let extracted_name = tokio::task::spawn_blocking(move || {
            extract_mod_name(&file_path_clone)
        }).await.unwrap_or(None);

        let display_name = extracted_name.unwrap_or_else(|| {
            Path::new(&file_name).file_stem().and_then(|s| s.to_str()).unwrap_or(&file_name).to_string()
        });
        
        let search_query = format!("{} minecraft mod", display_name).replace(" ", "+");
        let google_search_url = format!("https://www.google.com/search?q={}", search_query);

        manifest.push(ModEntry {
            file_name: file_name.clone(),
            target_dir: target_dir.clone(),
            sha512: sha512.clone(),
            download_url: None,
            project_name: None,
            version_number: None,
            source: "unknown".to_string(),
            page_url: Some(google_search_url),
            matched: false,
        });
    }

    // ═══ 自動保存: ゲームディレクトリに manifest を書き出す ═══
    let auto_save_path = game_dir.join("modlinker-manifest.json");
    let json_str = serde_json::to_string_pretty(&manifest).unwrap_or_default();
    if let Err(e) = tokio::fs::write(&auto_save_path, &json_str).await {
        emit_progress(&app, "warning", &format!("⚠ 自動保存失敗: {}", e), 0.95);
    } else {
        emit_progress(&app, "building", &format!("📄 マニフェスト自動保存: {}", auto_save_path.display()), 0.95);
    }

    emit_progress(&app, "done", "マニフェスト生成完了", 1.0);
    Ok(manifest)
}


// ── コマンド: 同期差分のチェック ──────────────────────────

#[tauri::command]
async fn check_sync_diff(app: AppHandle, game_path: String, manifest_json: String, skip_r2: bool) -> Result<Vec<DiffEntry>, String> {
    let game_dir = Path::new(&game_path);
    if !game_dir.exists() || !game_dir.is_dir() {
        return Err(format!("指定されたゲームディレクトリが無効です: {}", game_path));
    }

    let manifest_v2: ManifestV2 = ManifestV2::parse(&manifest_json)?;
    let manifest = manifest_v2.mods;

    if manifest.is_empty() {
        return Err("マニフェストが空です。".into());
    }

    emit_progress(&app, "scanning", "ローカル差分をチェック中...", 0.1);
    let mut diff_results = Vec::new();
    let total = manifest.len();

    for (i, entry) in manifest.iter().enumerate() {
        emit_progress(&app, "scanning", &format!("検証: {} ({}/{})", entry.file_name, i+1, total), 0.1 + (0.9 * (i as f64 / total as f64)));

        let target_path = game_dir.join(&entry.target_dir).join(&entry.file_name);
        let mut needs_download = true;
        if target_path.exists() {
            if let Ok(local_hash) = compute_sha512(&target_path).await {
                if local_hash == entry.sha512 { needs_download = false; }
            }
        }

        let action;
        if !needs_download {
            action = "up_to_date".to_string();
        } else if let Some(url) = &entry.download_url {
            let is_official = url.contains("cdn.modrinth.com") || url.contains("edge.forgecdn.net") || url.contains("mediafilez.forgecdn.net");
            if skip_r2 && !is_official {
                action = "skip".to_string();
            } else {
                action = "download".to_string();
            }
        } else if entry.page_url.is_some() {
            action = "manual".to_string();
        } else {
            action = "skip".to_string();
        }

        diff_results.push(DiffEntry {
            file_name: entry.file_name.clone(),
            target_dir: entry.target_dir.clone(),
            action,
            url: entry.download_url.clone(),
            page_url: entry.page_url.clone(),
            source: entry.source.clone(),
            sha512: Some(entry.sha512.clone()),
        });
    }

    emit_progress(&app, "done", "差分チェック完了", 1.0);
    Ok(diff_results)
}


// ── コマンド: ダウンロード実行 ──────────────────────────

#[tauri::command]
async fn execute_download(app: AppHandle, game_path: String, targets_json: String) -> Result<(), String> {
    let game_dir = Path::new(&game_path);
    let targets: Vec<DownloadTarget> = serde_json::from_str(&targets_json)
        .map_err(|e| format!("ダウンロードリスト解析失敗: {}", e))?;

    let dl_count = targets.len();
    if dl_count == 0 {
        emit_progress(&app, "done", "ダウンロード対象なし。", 1.0);
        return Ok(());
    }

    let client = reqwest::Client::builder().user_agent("modlinker/0.3.0").build().unwrap();

    for (i, target) in targets.iter().enumerate() {
        let progress = i as f64 / dl_count as f64;
        emit_progress(&app, "running", &format!("DL ({}/{}): {}", i+1, dl_count, target.file_name), progress);

        let target_path = game_dir.join(&target.target_dir).join(&target.file_name);
        if let Some(parent) = target_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| format!("フォルダ作成失敗: {}", e))?;
        }

        let mut res = client.get(&target.url).send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            emit_progress(&app, "error", &format!("✘ DL失敗 (HTTP {}): {}", res.status(), target.file_name), progress);
            continue;
        }

        let mut file = tokio::fs::File::create(target_path).await.map_err(|e| e.to_string())?;
        while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
    }

    emit_progress(&app, "done", &format!("完了! {} 個ダウンロード。", dl_count), 1.0);
    Ok(())
}

// ── コマンド: ファイル保存 ────────────────────────────────

#[tauri::command]
async fn save_manifest_to_file(path: String, content: String) -> Result<(), String> {
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("保存失敗: {}", e))
}

// ── コマンド: R2 自動アップロード ────────────────────────

#[tauri::command]
async fn upload_to_r2(
    app: AppHandle,
    file_path: String,
) -> Result<String, String> {
    emit_progress(&app, "upload", &format!("R2にアップロード準備中..."), 0.0);

    let path = Path::new(&file_path);
    if !path.exists() {
        return Err("ファイルが存在しません".into());
    }

    use tauri::Manager;
    let config_dir = app.path().app_config_dir()
        .map_err(|e| format!("設定ディレクトリ取得エラー: {}", e))?;
    let r2_config_path = config_dir.join("r2_config.json");
    if !r2_config_path.exists() {
        return Err("R2設定が見つかりません。設定画面からR2情報を保存してください。".into());
    }
    let r2_data = tokio::fs::read_to_string(&r2_config_path).await
        .map_err(|e| format!("R2設定読み込みエラー: {}", e))?;
    let r2_config: R2Config = serde_json::from_str(&r2_data)
        .map_err(|e| format!("R2設定パースエラー: {}", e))?;

    let file_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("jar");
    let original_name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
    let object_key = format!("{}-{}.{}", original_name, Uuid::new_v4(), file_ext);

    let endpoint_url = format!("https://{}.r2.cloudflarestorage.com", r2_config.account_id);
    
    let credentials = Credentials::new(
        r2_config.access_key,
        r2_config.secret_key,
        None,
        None,
        "modlinker",
    );

    let config = S3ConfigBuilder::new()
        .credentials_provider(credentials)
        .region(Region::new("auto"))
        .endpoint_url(endpoint_url)
        .build();

    let client = Client::from_conf(config);

    emit_progress(&app, "upload", &format!("{} をアップロード中...", original_name), 0.5);

    let body = ByteStream::from_path(&path).await.map_err(|e| format!("ファイル読み込みエラー: {}", e))?;

    client
        .put_object()
        .bucket(&r2_config.bucket)
        .key(&object_key)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("R2アップロードエラー: {:?}", e))?;

    let base = r2_config.public_url.trim_end_matches('/');
    let public_url = format!("{}/{}", base, object_key);

    emit_progress(&app, "upload", &format!("アップロード完了"), 1.0);
    Ok(public_url)
}

// ── コマンド: R2 設定管理 (セキュア保存) ─────────────────

#[derive(Clone, Serialize, Deserialize)]
struct R2Config {
    account_id: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    public_url: String,
}

fn get_r2_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let config_dir = app.path().app_config_dir()
        .map_err(|e| format!("設定ディレクトリ取得エラー: {}", e))?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("ディレクトリ作成エラー: {}", e))?;
    Ok(config_dir.join("r2_config.json"))
}

#[tauri::command]
async fn save_r2_config(app: AppHandle, config: R2Config) -> Result<(), String> {
    let path = get_r2_config_path(&app)?;
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("JSON変換エラー: {}", e))?;
    tokio::fs::write(&path, json).await
        .map_err(|e| format!("設定保存エラー: {}", e))
}

#[tauri::command]
async fn load_r2_config(app: AppHandle) -> Result<Option<R2Config>, String> {
    let path = get_r2_config_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let data = tokio::fs::read_to_string(&path).await
        .map_err(|e| format!("設定読み込みエラー: {}", e))?;
    let config: R2Config = serde_json::from_str(&data)
        .map_err(|e| format!("JSON解析エラー: {}", e))?;
    Ok(Some(config))
}

#[tauri::command]
async fn clear_r2_config(app: AppHandle) -> Result<(), String> {
    let path = get_r2_config_path(&app)?;
    if path.exists() {
        tokio::fs::remove_file(&path).await
            .map_err(|e| format!("設定削除エラー: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn read_file_content(path: String) -> Result<String, String> {
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("ファイル読み込みエラー: {}", e))
}

#[tauri::command]
fn open_browser(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

// ═══════════════════════════════════════════════════════════
//  v2.0.0 – 公式ランチャー完全連携 & スマート自動回収 コマンド群
// ═══════════════════════════════════════════════════════════

/// マニフェストからローダー情報 + チューニング + 手動DLターゲットを抽出 (UI表示用)
#[tauri::command]
fn detect_manifest_info(manifest_json: String) -> Result<serde_json::Value, String> {
    let m = ManifestV2::parse(&manifest_json)?;
    let manual_targets: Vec<serde_json::Value> = m
        .mods
        .iter()
        .filter(|e| e.download_url.is_none())
        .map(|e| {
            serde_json::json!({
                "file_name": e.file_name,
                "target_dir": e.target_dir,
                "sha512": e.sha512,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "format_version": m.format_version,
        "loader": m.loader,
        "tuning": m.tuning,
        "mod_count": m.mods.len(),
        "manual_targets": manual_targets,
    }))
}

/// MinecraftLauncher 起動チェック (UIで事前警告を表示するため)
#[tauri::command]
fn check_launcher_running_cmd() -> launcher::profiles::LauncherProcCheck {
    launcher::profiles::check_launcher_running()
}

/// Java ランタイム探索 (内蔵ランタイム優先)
#[tauri::command]
async fn resolve_java(required_major: Option<u8>) -> Result<Option<launcher::java::JavaRuntime>, String> {
    let major = required_major.unwrap_or(21);
    Ok(match launcher::installer::find_java_runtime(major).await {
        Some(rt) => Some(rt),
        None => launcher::installer::find_best_java().await,
    })
}

/// 型安全 JVM プラン生成 (UIの即時プレビュー用)
/// ※ 生のJVM引数文字列は受け取らない — JvmTuning 構造体のみ
#[tauri::command]
fn plan_jvm_args(
    tuning_json: Option<serde_json::Value>,
    zgc_enabled: bool,
    java_major: Option<u8>,
) -> Result<JvmPlan, String> {
    let tuning: crate::manifest::JvmTuning = match tuning_json {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("tuning の解析失敗: {}", e))?,
        None => Default::default(),
    };
    let major = java_major.unwrap_or(tuning.recommended_java);
    Ok(jvm::build_jvm_plan(&tuning, major, zgc_enabled, jvm::physical_ram_mb()))
}

/// 起動失敗時の自動フォールバック: G1GC 版プランを再生成 (公式 Java 遷移導線)
#[tauri::command]
fn plan_jvm_fallback(
    tuning_json: Option<serde_json::Value>,
    java_major: Option<u8>,
) -> Result<JvmPlan, String> {
    let tuning: crate::manifest::JvmTuning = match tuning_json {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("tuning の解析失敗: {}", e))?,
        None => Default::default(),
    };
    let major = java_major.unwrap_or(tuning.recommended_java);
    Ok(jvm::fallback_g1_plan(&tuning, major, jvm::physical_ram_mb()))
}

/// ローダーバージョン事前解決 (UI表示用)
#[tauri::command]
async fn resolve_loader_version(
    loader: LoaderInfo,
) -> Result<String, String> {
    launcher::installer::resolve_loader_versions(&loader).await
}

/// 【ワンクリック同期】並行パイプライン実行 (ローダー導入 ∥ MODダウンロード → 原子的合体 → プロファイル追記)
#[tauri::command]
async fn run_launcher_sync(
    app: AppHandle,
    game_path: String,
    manifest_json: String,
    options: LauncherSyncOptions,
) -> Result<launcher::pipeline::SyncSummary, String> {
    let game_root = PathBuf::from(&game_path);
    if !game_root.exists() || !game_root.is_dir() {
        return Err(format!("指定されたゲームディレクトリが無効です: {}", game_path));
    }
    let manifest = ManifestV2::parse(&manifest_json)?;
    launcher::pipeline::run_launcher_sync(app, game_root, manifest, options).await
}

/// ダウンロードフォルダ解決 (設定 > ブラウザPreferences > OS標準)
#[tauri::command]
fn resolve_download_dir_cmd(preferred: Option<String>) -> browser_paths::ResolvedDownloadDir {
    browser_paths::resolve_download_dir(preferred)
}

/// 【自動回収】監視開始 (待機モーダルが開いている間のみ)
#[tauri::command]
fn start_download_watcher(
    app: AppHandle,
    targets_json: String,
    watch_dir: Option<String>,
    dest_root: String,
    manager: tauri::State<'_, watcher::WatcherManager>,
) -> Result<serde_json::Value, String> {
    let targets: Vec<WatchTarget> = serde_json::from_str(&targets_json)
        .map_err(|e| format!("監視対象の解析失敗: {}", e))?;
    if targets.is_empty() {
        return Err("監視対象MODがありません。".into());
    }

    let dir = match watch_dir.filter(|s| !s.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            // ブラウザ Preferences → OS標準 Downloads の順で自動特定
            let resolved = browser_paths::resolve_download_dir(None);
            PathBuf::from(resolved.path)
        }
    };

    let session_id = format!("watch-{}", uuid::Uuid::new_v4().simple());
    let dir_str = dir.to_string_lossy().to_string();
    watcher::start_watcher(app, session_id.clone(), targets, dir, PathBuf::from(dest_root), manager.inner())?;

    Ok(serde_json::json!({
        "session_id": session_id,
        "watch_dir": dir_str,
    }))
}

/// 【自動回収】監視停止・破棄 (キャンセル時。常時監視は絶対禁止)
#[tauri::command]
fn stop_download_watcher(
    session_id: String,
    manager: tauri::State<'_, watcher::WatcherManager>,
) -> Result<(), String> {
    watcher::stop_watcher(&session_id, manager.inner())
}

/// 【フォールバック】ワンショット一括ハッシュ照合スキャン
#[tauri::command]
async fn scan_download_dir_once(
    targets_json: String,
    watch_dir: Option<String>,
    dest_root: String,
) -> Result<watcher::ScanResult, String> {
    let targets: Vec<WatchTarget> = serde_json::from_str(&targets_json)
        .map_err(|e| format!("監視対象の解析失敗: {}", e))?;
    let dir = match watch_dir.filter(|s| !s.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let resolved = browser_paths::resolve_download_dir(None);
            PathBuf::from(resolved.path)
        }
    };
    watcher::scan_once(dir, PathBuf::from(dest_root), targets).await
}

/// 【フォールバック】ファイル直接選択 / ドラッグ&ドロップで配置 (複数一括)
#[tauri::command]
async fn place_files_from_paths(
    paths: Vec<String>,
    targets_json: String,
    dest_root: String,
) -> Result<watcher::PlaceResult, String> {
    let targets: Vec<WatchTarget> = serde_json::from_str(&targets_json)
        .map_err(|e| format!("監視対象の解析失敗: {}", e))?;
    let path_bufs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    watcher::place_files(path_bufs, targets, PathBuf::from(dest_root)).await
}

// ── エントリポイント ─────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(watcher::WatcherManager::new())
        .invoke_handler(tauri::generate_handler![
            sync_mods,
            check_sync_diff,
            execute_download,
            save_manifest_to_file,
            upload_to_r2,
            save_r2_config,
            load_r2_config,
            clear_r2_config,
            read_file_content,
            open_browser,
            // ── v2.0.0 ──
            detect_manifest_info,
            check_launcher_running_cmd,
            resolve_java,
            plan_jvm_args,
            plan_jvm_fallback,
            resolve_loader_version,
            run_launcher_sync,
            resolve_download_dir_cmd,
            start_download_watcher,
            stop_download_watcher,
            scan_download_dir_once,
            place_files_from_paths
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
