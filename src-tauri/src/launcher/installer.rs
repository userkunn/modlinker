// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – MODローダー公式インストーラーのサイレント実行代行
//
//  自前で複雑な起動JSONを生成せず、Fabric / Forge / NeoForge の
//  公式インストーラーを取得し、内蔵Javaでバックグラウンド・サイレント実行する。
// ═══════════════════════════════════════════════════════════

use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::manifest::{LoaderInfo, LoaderKind};
use super::java;

/// インストール進捗イベント
#[derive(Clone, Serialize)]
pub struct InstallerEvent {
    pub stage: String,      // "resolve" | "download" | "install" | "done" | "error"
    pub message: String,
}

fn emit(app: &tauri::AppHandle, stage: &str, msg: &str) {
    let _ = app.emit("installer-progress", InstallerEvent {
        stage: stage.to_string(),
        message: msg.to_string(),
    });
}

const USER_AGENT: &str = "modlinker/2.0.0 (github.com/userkunn/modlinker)";

// ── バージョン解決 ───────────────────────────────────────

/// Fabric メタからインストーラー最新版とローダー最新版を取得
async fn resolve_fabric(mc_version: &str) -> Result<(String, String), String> {
    let client = reqwest::Client::new();

    // インストーラー最新版
    #[derive(Deserialize)]
    struct FabricInstallerMeta { version: String }
    let installer_ver = client
        .get("https://meta.fabricmc.net/v2/versions/installer")
        .send().await.map_err(|e| e.to_string())?
        .json::<Vec<FabricInstallerMeta>>().await.map_err(|e| e.to_string())?
        .first().map(|v| v.version.clone())
        .ok_or("Fabric installer version not found")?;

    // ローダー最新版 (MCバージョン互換)
    #[derive(Deserialize)]
    struct FabricLoaderEntry { loader: FabricLoaderInner }
    #[derive(Deserialize)]
    struct FabricLoaderInner { version: String }
    let loader_ver = client
        .get(format!("https://meta.fabricmc.net/v2/versions/loader/{}", mc_version))
        .send().await.map_err(|e| e.to_string())?
        .json::<Vec<FabricLoaderEntry>>().await.map_err(|e| e.to_string())?
        .first().map(|v| v.loader.version.clone())
        .ok_or(format!("Fabric loader not available for MC {}", mc_version))?;

    Ok((installer_ver, loader_ver))
}

/// Forge バージョン解決 (promotions_slim.json → maven-metadata.xml フォールバック)
async fn resolve_forge(mc_version: &str) -> Result<String, String> {
    let client = reqwest::Client::new();

    #[derive(Deserialize)]
    struct Promos { promos: std::collections::HashMap<String, String> }
    if let Ok(resp) = client
        .get("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")
        .send().await
    {
        if let Ok(p) = resp.json::<Promos>().await {
            for key in [format!("{}-recommended", mc_version), format!("{}-latest", mc_version)] {
                if let Some(v) = p.promos.get(&key) {
                    return Ok(v.clone());
                }
            }
        }
    }

    // maven-metadata から該当 MC の最新を探す
    let md = client
        .get("https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml")
        .send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;
    // 簡易XML走査 (serde_xml に依存しない)
    let mut found: Option<String> = None;
    let mut rest = md.as_str();
    while let Some(start) = rest.find("<version>") {
        let after = &rest[start + 9..];
        let end = after.find("</version>").unwrap_or(after.len());
        let ver = &after[..end];
        if ver.starts_with(&format!("{}-", mc_version)) {
            found = Some(ver.to_string());
        }
        rest = &after[end..];
    }
    found.ok_or(format!("Forge not available for MC {}", mc_version))
}

/// NeoForge バージョン解決
async fn resolve_neoforge(mc_version: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let text = client
        .get("https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge")
        .send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let versions = json.get("versions").and_then(|v| v.as_array()).ok_or("neoforge version list unavailable")?;

    // "1.21.1" → "1.21.1.xxx" 形式の最新。mc_version 完全一致 (例: 1.21) も許容
    let prefix = format!("{}.", mc_version);
    let mut best: Option<String> = None;
    for v in versions.iter().filter_map(|v| v.as_str()) {
        if v == mc_version {
            return Ok(v.to_string());
        }
        if v.starts_with(&prefix) {
            best = Some(v.to_string());
        }
    }
    best.ok_or(format!("NeoForge not available for MC {}", mc_version))
}

// ── インストーラーjar取得 ────────────────────────────────

async fn download_installer_jar(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
) -> Result<(), String> {
    let resp = client.get(url).send().await.map_err(|e| format!("ダウンロード失敗 {}: {}", url, e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} {}", resp.status(), url));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    tokio::fs::write(dest, &bytes).await.map_err(|e| e.to_string())?;
    Ok(())
}

// ── サイレント実行 ───────────────────────────────────────

/// 公式インストーラーをサイレント実行して gameDir (staging) にローダーを導入する。
///
/// - Fabric:  `java -jar fabric-installer.jar client -mcVersion X -loaderVersion Y -dir <dir> -noprofile`
/// - Forge:   `java -jar forge-installer.jar --installClient <dir>`
/// - NeoForge: `java -jar neoforge-installer.jar --installClient <dir>`
///
/// `noprofile=true` の場合、launcher_profiles.json には触れない
/// (プロファイル管理は profiles.rs が専任で行う — 安全装置の二重化)。
pub async fn install_loader(
    app: &tauri::AppHandle,
    loader: &LoaderInfo,
    java_exe: &Path,
    _game_dir: &Path,
    staging_dir: &Path,
) -> Result<InstalledLoader, String> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build().map_err(|e| e.to_string())?;

    // サイレント実行の検証: MCバージョン形式チェック (引数インジェクション遮断)
    if !crate::manifest::ManifestV2::validate_version(&loader.mc_version) {
        return Err(format!("不正な MC バージョン形式: {}", loader.mc_version));
    }
    if let Some(lv) = &loader.loader_version {
        if !crate::manifest::ManifestV2::validate_version(lv) {
            return Err(format!("不正なローダーバージョン形式: {}", lv));
        }
    }

    // staging 内に gameDir を物理作成
    tokio::fs::create_dir_all(staging_dir).await.map_err(|e| e.to_string())?;

    let (jar_url, jar_file): (String, String) = match loader.kind {
        LoaderKind::Vanilla => return Err("Vanilla ではインストーラー不要です".into()),
        LoaderKind::Fabric => {
            emit(app, "resolve", "Fabric: メタデータを解決中...");
            let (installer_ver, loader_ver) = resolve_fabric(&loader.mc_version).await?;
            let lv = loader.loader_version.clone().unwrap_or(loader_ver);
            (
                format!("https://maven.fabricmc.net/net/fabricmc/fabric-installer/{}/fabric-installer-{}.jar", installer_ver, installer_ver),
                lv,
            )
        }
        LoaderKind::Forge => {
            emit(app, "resolve", "Forge: バージョンを解決中...");
            let ver = match &loader.loader_version {
                Some(v) if crate::manifest::ManifestV2::validate_version(v) => v.clone(),
                _ => resolve_forge(&loader.mc_version).await?,
            };
            (
                format!("https://maven.minecraftforge.net/net/minecraftforge/forge/{}-{}/forge-{}-{}-installer.jar", loader.mc_version, ver, loader.mc_version, ver),
                format!("{}-{}", loader.mc_version, ver),
            )
        }
        LoaderKind::NeoForge => {
            emit(app, "resolve", "NeoForge: バージョンを解決中...");
            let ver = match &loader.loader_version {
                Some(v) if crate::manifest::ManifestV2::validate_version(v) => v.clone(),
                _ => resolve_neoforge(&loader.mc_version).await?,
            };
            (
                format!("https://maven.neoforged.net/releases/net/neoforged/neoforge/{}/neoforge-{}-installer.jar", ver, ver),
                ver,
            )
        }
    };

    // インストーラー jar を staging に取得
    let jar_path = staging_dir.join("modlinker-installer.jar");
    emit(app, "download", "公式インストーラーを取得中...");
    download_installer_jar(&client, &jar_url, &jar_path).await?;

    // ── サイレント実行 ──
    let mut cmd = tokio::process::Command::new(java_exe);
    cmd.current_dir(staging_dir)
        .arg("-jar")
        .arg(&jar_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // インストーラーは GUI を出さないヘッドレスモードで実行
        .env("JAVA_TOOL_OPTIONS", "-Djava.awt.headless=true");

    let version_id: String;
    match loader.kind {
        LoaderKind::Fabric => {
            // fabric-installer のサイレントクライアントモード
            // -noprofile: launcher_profiles.json への書き込みを抑制 (プロファイルは ModLinker 側で管理)
            cmd.arg("client")
                .arg("-mcVersion").arg(&loader.mc_version)
                .arg("-loaderVersion").arg(&jar_file)
                .arg("-dir").arg(staging_dir)
                .arg("-noprofile");
            version_id = format!("fabric-loader-{}-{}", jar_file, loader.mc_version);
        }
        LoaderKind::Forge | LoaderKind::NeoForge => {
            // Forge / NeoForge installer のサイレントモード
            cmd.arg("--installClient").arg(staging_dir);
            let prefix = if loader.kind == LoaderKind::Forge { "forge" } else { "neoforge" };
            version_id = format!("{}-{}", prefix, jar_file);
        }
        LoaderKind::Vanilla => unreachable!(),
    }

    emit(app, "install", "インストーラーをサイレント実行中 (バックグラウンド)...");
    let output = cmd.output().await.map_err(|e| format!("インストーラー起動失敗: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        return Err(format!(
            "インストーラーが失敗しました (exit {}):\n{}\n{}",
            output.status.code().unwrap_or(-1),
            stdout.chars().take(2000).collect::<String>(),
            stderr.chars().take(2000).collect::<String>()
        ));
    }

    // ── 検証: versions/<id>/<id>.json が生成されたか ──
    let versions_dir = staging_dir.join("versions").join(&version_id);
    let json_file = versions_dir.join(format!("{}.json", version_id));
    let mut installed = json_file.exists();
    if !installed {
        if let Ok(mut rd) = tokio::fs::read_dir(staging_dir.join("versions")).await {
            while let Ok(Some(e)) = rd.next_entry().await {
                if e.path().is_dir() {
                    installed = true;
                    break;
                }
            }
        }
    }

    if !installed {
        return Err(format!(
            "インストール検証に失敗しました (versions/ に {} が見つかりません)",
            version_id
        ));
    }

    // インストーラー jar を掃除
    let _ = tokio::fs::remove_file(&jar_path).await;

    emit(app, "done", &format!("ローダー導入完了: {}", version_id));
    Ok(InstalledLoader {
        version_id,
    })
}

/// インストール結果
pub struct InstalledLoader {
    /// 例: "fabric-loader-0.16.9-1.21.1"
    pub version_id: String,
}

/// ローダーバージョンを事前解決するだけのユーティリティ (UI表示用)
pub async fn resolve_loader_versions(loader: &LoaderInfo) -> Result<String, String> {
    match loader.kind {
        LoaderKind::Vanilla => Ok("vanilla".to_string()),
        LoaderKind::Fabric => {
            let (_, lv) = resolve_fabric(&loader.mc_version).await?;
            Ok(lv)
        }
        LoaderKind::Forge => resolve_forge(&loader.mc_version).await,
        LoaderKind::NeoForge => resolve_neoforge(&loader.mc_version).await,
    }
}

/// MC バージョンから推奨 Java メジャーバージョンを算出
pub fn recommended_java_for_mc(mc_version: &str) -> u8 {
    let mut it = mc_version.split('.');
    let major: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let minor: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    match (major, minor) {
        (_, m) if m >= 21 => 21,          // 1.21+
        (_, m) if m >= 20 => {           // 1.20.5+ は Java 21
            let patch: u32 = it.next().and_then(|s| s.split(['-', '+']).next()).and_then(|s| s.parse().ok()).unwrap_or(0);
            if m > 20 || patch >= 5 { 21 } else { 17 }
        }
        (_, m) if m >= 18 => 17,          // 1.18 ~ 1.20.4
        (_, m) if m == 17 => 16,          // 1.17
        _ => 8,                            // 1.16.5 以下
    }
}

// java モジュールの re-export (lib.rs から参照しやすく)
pub use java::{find_best_java, find_java_runtime};
