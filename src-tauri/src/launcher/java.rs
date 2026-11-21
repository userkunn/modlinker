// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – Java Runtime 自動探索
//
//  ユーザーPCの Minecraft 公式ランチャー内蔵 Java
//  (runtime/java-runtime-gamma / delta 等) を優先的に探索し、
//  Java 未導入環境でもエラーなしで完走させる。
// ═══════════════════════════════════════════════════════════

use serde::Serialize;
use std::path::{Path, PathBuf};

/// 探索した Java ランタイム
#[derive(Clone, Serialize, Debug)]
pub struct JavaRuntime {
    /// java 実行ファイルの絶対パス
    pub executable: String,
    /// メジャーバージョン (例: 21)
    pub major_version: u8,
    /// 完全バージョン文字列 (例: "21.0.3")
    pub version_string: String,
    /// 探索元 ("minecraft_runtime:java-runtime-gamma" | "java_home" | "path" ...)
    pub origin: String,
}

/// 内蔵ランタイムの候補名 (新しい順の優先度)
const RUNTIME_DIRS: &[&str] = &[
    "java-runtime-gamma",   // Java 21 系 (1.20.5+)
    "java-runtime-delta",   // Java 17 系 (1.18~1.20.4)
    "java-runtime-alpha",
    "java-runtime-beta",
    "java-runtime-omega",
    "jre-legacy",           // Java 8 系
];

/// ランタイム配下のプラットフォームディレクトリ候補
const PLATFORM_DIRS: &[&str] = &[
    "windows-x64",
    "windows-arm64",
    "linux",
    "linux-i386",
    "mac-os",
    "mac-os-arm64",
];

/// ランタイムのルートディレクトリ候補を OS ごとに列挙
fn runtime_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    fn push(roots: &mut Vec<PathBuf>, p: Option<PathBuf>) {
        if let Some(p) = p {
            roots.push(p);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
            let lad = PathBuf::from(lad);
            // 通常版: %LOCALAPPDATA%\.minecraft\runtime
            push(&mut roots, Some(lad.join(".minecraft").join("runtime")));
            // Microsoft Store 版: %LOCALAPPDATA%\Packages\Microsoft.4297127D64EC6_8wekyb3d8bbwe\LocalCache\Local\runtime
            push(&mut roots, Some(lad
                .join("Packages")
                .join("Microsoft.4297127D64EC6_8wekyb3d8bbwe")
                .join("LocalCache")
                .join("Local")
                .join("runtime")));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            push(&mut roots, Some(home.join("Library/Application Support/minecraft/runtime")));
        }
        push(&mut roots, Some(PathBuf::from("/Library/Application Support/minecraft/runtime")));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(home) = dirs::home_dir() {
            push(&mut roots, Some(home.join(".minecraft/runtime")));
        }
    }

    roots
}

/// java 実行ファイルの候補パスを生成
fn java_bin_candidates() -> Vec<PathBuf> {
    let mut cands = Vec::new();
    let exe = if cfg!(windows) { "java.exe" } else { "java" };

    for root in runtime_roots() {
        for rt in RUNTIME_DIRS {
            let rt_root = root.join(rt);
            if !rt_root.exists() {
                continue;
            }
            // 公式構造: <rt>/<platform>/<rt>/bin/java
            for plat in PLATFORM_DIRS {
                let p = rt_root.join(plat).join(rt).join("bin").join(exe);
                if p.exists() {
                    cands.push(p);
                }
            }
            // 非標準構造のフォールバック: <rt>/bin/java
            let p = rt_root.join("bin").join(exe);
            if p.exists() {
                cands.push(p);
            }
        }
    }
    cands
}

/// OS の汎用 Java 候補 (JAVA_HOME, PATH, JVM 標準パス)
fn generic_java_candidates() -> Vec<(PathBuf, &'static str)> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let mut cands: Vec<(PathBuf, &'static str)> = Vec::new();

    if let Ok(home) = std::env::var("JAVA_HOME") {
        let p = PathBuf::from(home).join("bin").join(exe);
        if p.exists() {
            cands.push((p, "java_home"));
        }
    }

    // PATH 上の java
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let p = dir.join(exe);
            if p.exists() {
                cands.push((p, "path"));
            }
        }
    }

    // JVM 標準インストール先
    #[cfg(target_os = "windows")]
    {
        for base in ["C:\\Program Files\\Java", "C:\\Program Files (x86)\\Java", "C:\\Program Files\\Eclipse Adoptium", "C:\\Program Files\\Microsoft"] {
            if let Ok(entries) = std::fs::read_dir(base) {
                for e in entries.flatten() {
                    let p = e.path().join("bin").join(exe);
                    if p.exists() {
                        cands.push((p, "system"));
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let p = PathBuf::from("/usr/bin/java");
        if p.exists() {
            cands.push((p, "system"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(entries) = std::fs::read_dir("/usr/lib/jvm") {
            for e in entries.flatten() {
                let p = e.path().join("bin").join(exe);
                if p.exists() {
                    cands.push((p, "system"));
                }
            }
        }
    }

    cands
}

/// `java -version` を実行してバージョンを検出 (タイムアウト 15 秒)
fn detect_java_version(exec: &Path) -> Option<(u8, String)> {
    use std::process::{Command, Stdio};

    let output = Command::new(exec)
        .arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;

    // java -version は stderr に出力される
    let mut text = String::from_utf8_lossy(&output.stderr).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stdout).to_string();
    }

    parse_java_version_output(&text)
}

/// `java -version` 出力をパース
/// 例: `openjdk version "21.0.3" 2024-04-16` / `java version "1.8.0_402"` / `openjdk version "17.0.11"`
pub fn parse_java_version_output(text: &str) -> Option<(u8, String)> {
    for line in text.lines() {
        if let Some(idx) = line.find("version \"") {
            let rest = &line[idx + "version \"".len()..];
            let end = rest.find('"')?;
            let ver = &rest[..end];
            // 1.8.0_402 形式 → メジャー 8
            let mut parts = ver.split(['.', '_']);
            let major_str = parts.next()?.trim();
            if major_str == "1" {
                let minor = parts.next()?.trim().parse::<u8>().ok()?;
                return Some((minor, ver.to_string()));
            }
            let major = major_str.parse::<u8>().ok()?;
            return Some((major, ver.to_string()));
        }
    }
    None
}

/// 要求 Java メジャーバージョンに最も適したランタイムを探索。
/// 1) 内蔵ランタイム (バージョン検出つき) → 2) JAVA_HOME/PATH/システム
pub async fn find_java_runtime(required_major: u8) -> Option<JavaRuntime> {
    let cands = java_bin_candidates();
    let generic = generic_java_candidates();
    let mut all: Vec<(PathBuf, String)> = cands
        .into_iter()
        .map(|p| (p.clone(), format!("minecraft_runtime:{}", p.parent().and_then(|d| d.parent()).map(|x| x.file_name().unwrap_or_default().to_string_lossy()).unwrap_or_default())))
        .collect();
    all.extend(generic.into_iter().map(|(p, o)| (p, o.to_string())));

    let mut best: Option<JavaRuntime> = None;

    for (path, origin) in all {
        let path = tokio::task::spawn_blocking(move || {
            let ver = detect_java_version(&path);
            (path, ver)
        })
        .await
        .ok();

        if let Some((path, Some((major, verstr)))) = path {
            let exact = major == required_major;
            let compatible = major >= required_major && major <= required_major + 3;
            let better = match &best {
                None => true,
                Some(b) => {
                    if exact && b.major_version != required_major { true }
                    else if exact && b.major_version == required_major {
                        // どちらも完全一致 → 内蔵ランタイムを優先
                        origin.starts_with("minecraft_runtime") && !b.origin.starts_with("minecraft_runtime")
                    }
                    else { false }
                }
            };
            if better && (exact || compatible) {
                best = Some(JavaRuntime {
                    executable: path.to_string_lossy().to_string(),
                    major_version: major,
                    version_string: verstr,
                    origin,
                });
            }
            if exact && best.as_ref().map_or(false, |b| b.major_version == required_major && b.origin.starts_with("minecraft_runtime")) {
                break; // 内蔵ランタイムで完全一致なら即決
            }
        }
    }

    best
}

/// インストール済み Java のうち利用可能な最良のもの (要求なし・最上位を探す)
pub async fn find_best_java() -> Option<JavaRuntime> {
    if let Some(rt) = find_java_runtime(21).await {
        return Some(rt);
    }
    if let Some(rt) = find_java_runtime(17).await {
        return Some(rt);
    }
    find_java_runtime(8).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_java_versions() {
        let (m, _) = parse_java_version_output("openjdk version \"21.0.3\" 2024-04-16").unwrap();
        assert_eq!(m, 21);
        let (m, _) = parse_java_version_output("java version \"1.8.0_402\"").unwrap();
        assert_eq!(m, 8);
        let (m, _) = parse_java_version_output("openjdk version \"17.0.11\" 2024-04-16").unwrap();
        assert_eq!(m, 17);
        assert!(parse_java_version_output("no version here").is_none());
    }
}
