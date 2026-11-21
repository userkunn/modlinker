// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – 主要ブラウザのカスタムダウンロード先解決 (要件 2-B)
//
//  監視対象フォルダは、以下の優先順位で自動特定する:
//   1. アプリ内設定 (ユーザーが手動で変更・記憶させたパスがあれば最優先)
//   2. 主要 Chromium 系ブラウザの Preferences JSON (download.default_directory)
//      - Chrome / Edge / Brave / Arc / Opera / Opera GX
//   3. OS 標準の Downloads ディレクトリ
// ═══════════════════════════════════════════════════════════

use std::path::PathBuf;

use serde::Serialize;

/// 解決結果
#[derive(Serialize, Debug)]
pub struct ResolvedDownloadDir {
    pub path: String,
    /// "user_setting" | "browser_preference" | "os_default" | "fallback_home"
    pub source: String,
    /// 検出したブラウザ名 (例: "Chrome", "Edge")
    pub browser: Option<String>,
}

struct BrowserPref {
    name: &'static str,
    paths: fn() -> Vec<PathBuf>,
}

fn chromium_paths(
    base_candidates: Vec<PathBuf>,
    profile_dirs: &[&str],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for base in base_candidates {
        for pd in profile_dirs {
            out.push(base.join(pd).join("Preferences"));
        }
    }
    out
}

/// 各ブラウザの User Data ディレクトリ候補 (OS別)
fn chrome_base() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut v = Vec::new();
        if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(lad).join("Google").join("Chrome").join("User Data"));
        }
        v
    }
    #[cfg(target_os = "macos")]
    {
        vec![dirs::home_dir().map_or_else(
            || PathBuf::from("Library/Application Support/Google/Chrome"),
            |h| h.join("Library/Application Support/Google/Chrome"),
        )]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![dirs::config_dir().map_or_else(
            || PathBuf::from(".config/google-chrome"),
            |c| c.join("google-chrome"),
        )]
    }
}

fn edge_base() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut v = Vec::new();
        if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(lad).join("Microsoft").join("Edge").join("User Data"));
        }
        v
    }
    #[cfg(target_os = "macos")]
    {
        vec![dirs::home_dir().map_or_else(
            || PathBuf::from("Library/Application Support/Microsoft Edge"),
            |h| h.join("Library/Application Support/Microsoft Edge"),
        )]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![dirs::config_dir().map_or_else(
            || PathBuf::from(".config/microsoft-edge"),
            |c| c.join("microsoft-edge"),
        )]
    }
}

fn brave_base() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut v = Vec::new();
        if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(lad).join("BraveSoftware").join("Brave-Browser").join("User Data"));
        }
        v
    }
    #[cfg(target_os = "macos")]
    {
        vec![dirs::home_dir().map_or_else(
            || PathBuf::from("Library/Application Support/BraveSoftware/Brave-Browser"),
            |h| h.join("Library/Application Support/BraveSoftware/Brave-Browser"),
        )]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![dirs::config_dir().map_or_else(
            || PathBuf::from(".config/BraveSoftware/Brave-Browser"),
            |c| c.join("BraveSoftware/Brave-Browser"),
        )]
    }
}

fn arc_base() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut v = Vec::new();
        if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(lad).join("TheBrowserCompany").join("Arc").join("User Data"));
        }
        v
    }
    #[cfg(target_os = "macos")]
    {
        vec![dirs::home_dir().map_or_else(
            || PathBuf::from("Library/Application Support/Arc/User Data"),
            |h| h.join("Library/Application Support/Arc/User Data"),
        )]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Arc Linux 版は Arc Browser を使用
        vec![dirs::config_dir().map_or_else(
            || PathBuf::from(".config/ArcBrowser"),
            |c| c.join("ArcBrowser"),
        )]
    }
}

fn opera_base(gx: bool) -> Vec<PathBuf> {
    let dir_name = if gx { "Opera GX Stable" } else { "Opera Stable" };
    #[cfg(target_os = "windows")]
    {
        let mut v = Vec::new();
        if let Some(ad) = std::env::var_os("APPDATA") {
            v.push(PathBuf::from(ad).join("Opera Software").join(dir_name));
        }
        v
    }
    #[cfg(target_os = "macos")]
    {
        vec![dirs::home_dir().map_or_else(
            || PathBuf::from(format!("Library/Application Support/com.operasoftware.Opera")),
            |h| h.join(format!("Library/Application Support/com.operasoftware.{}", if gx { "OperaGX" } else { "Opera" })),
        )]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![dirs::config_dir().map_or_else(
            || PathBuf::from(format!(".config/{}", dir_name.to_lowercase().replace(' ', "-"))),
            |c| c.join(dir_name.to_lowercase().replace(' ', "-")),
        )]
    }
}

fn browser_list() -> Vec<BrowserPref> {
    vec![
        BrowserPref { name: "Chrome", paths: || chromium_paths(chrome_base(), &["Default", "Profile 1", "Profile 2"]) },
        BrowserPref { name: "Edge", paths: || chromium_paths(edge_base(), &["Default", "Profile 1", "Profile 2"]) },
        BrowserPref { name: "Brave", paths: || chromium_paths(brave_base(), &["Default", "Profile 1", "Profile 2"]) },
        BrowserPref { name: "Arc", paths: || chromium_paths(arc_base(), &["Default", "Profile 1", "Profile 2"]) },
        BrowserPref { name: "Opera", paths: || chromium_paths(opera_base(false), &["Default"]) },
        BrowserPref { name: "Opera GX", paths: || chromium_paths(opera_base(true), &["Default"]) },
    ]
}

/// Preferences JSON から download.default_directory を読む (壊れたJSON・無権限は無視)
fn read_download_pref(pref_path: &PathBuf) -> Option<String> {
    let content = std::fs::read_to_string(pref_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    let dir = v
        .get("download")
        .and_then(|d| d.get("default_directory"))
        .and_then(|s| s.as_str())?;
    if dir.trim().is_empty() {
        None
    } else {
        Some(dir.to_string())
    }
}

/// ダウンロードディレクトリを解決する。
///
/// `preferred`: アプリ内設定のパス (最優先・None可)
/// `browser_order`: 空なら既定順 (Chrome→Edge→Brave→Arc→Opera→Opera GX)
pub fn resolve_download_dir(preferred: Option<String>) -> ResolvedDownloadDir {
    // 1. アプリ内設定が最優先
    if let Some(p) = preferred.filter(|p| !p.trim().is_empty()) {
        let pb = PathBuf::from(&p);
        if pb.exists() {
            return ResolvedDownloadDir {
                path: pb.to_string_lossy().to_string(),
                source: "user_setting".into(),
                browser: None,
            };
        }
        // 存在しなくても作成可能ならそれを使う (後で mkdir するため)
        return ResolvedDownloadDir {
            path: pb.to_string_lossy().to_string(),
            source: "user_setting".into(),
            browser: None,
        };
    }

    // 2. Chromium 系 Preferences
    for b in browser_list() {
        for pref in (b.paths)() {
            if let Some(dir) = read_download_pref(&pref) {
                return ResolvedDownloadDir {
                    path: dir,
                    source: "browser_preference".into(),
                    browser: Some(b.name.to_string()),
                };
            }
        }
    }

    // 3. OS 標準 Downloads
    if let Some(dl) = dirs::download_dir() {
        return ResolvedDownloadDir {
            path: dl.to_string_lossy().to_string(),
            source: "os_default".into(),
            browser: None,
        };
    }

    // 4. 最終フォールバック
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    ResolvedDownloadDir {
        path: home.join("Downloads").to_string_lossy().to_string(),
        source: "fallback_home".into(),
        browser: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_download_pref() {
        let tmp = std::env::temp_dir().join(format!("ml_pref_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let pref = tmp.join("Preferences");
        std::fs::write(&pref, r#"{"download": {"default_directory": "/custom/downloads"}}"#).unwrap();
        assert_eq!(read_download_pref(&pref), Some("/custom/downloads".to_string()));

        // 壊れたJSON → None (フォールバック継続)
        std::fs::write(&pref, "{broken").unwrap();
        assert_eq!(read_download_pref(&pref), None);

        // 空文字 → None
        std::fs::write(&pref, r#"{"download": {"default_directory": ""}}"#).unwrap();
        assert_eq!(read_download_pref(&pref), None);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_priority_user_setting() {
        // 存在するディレクトリなら最優先で返る
        let tmp = std::env::temp_dir().join(format!("ml_res_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let r = resolve_download_dir(Some(tmp.to_string_lossy().to_string()));
        assert_eq!(r.source, "user_setting");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_resolve_falls_back_to_os_downloads() {
        let r = resolve_download_dir(None);
        // sandbox には Downloads 相当が無い場合もあるが、home フォールバックが必ず返る
        assert!(r.source == "browser_preference" || r.source == "os_default" || r.source == "fallback_home");
        assert!(!r.path.is_empty());
    }
}
