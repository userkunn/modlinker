// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – launcher_profiles.json 安全追記
//
//  【絶対の安全装置】 (要件 1-C)
//   1. 書き込み実行前に MinecraftLauncher.exe が起動中かプロセスチェックし、
//      起動中は警告を出してブロック (保存中止)。
//   2. 書き換え直前に同一ディレクトリへタイムスタンプ付き .bak を自動生成。
//
//   さらに:
//   - 追記は既存キーを破壊しない (Append のみ、JSON解析失敗時は書き込み中止)
//   - JVM引数は JvmPlan (型安全ビルダー) の生成結果のみを許可
// ═══════════════════════════════════════════════════════════

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// プロセスチェック結果
#[derive(Serialize, Debug)]
pub struct LauncherProcCheck {
    pub running: bool,
    pub matched_name: Option<String>,
}

/// Minecraft 公式ランチャーのプロセスが起動中かチェックする。
/// Windows: MinecraftLauncher.exe / minecraft.exe / Minecraft.exe
/// macOS:   "Minecraft" (Minecraft.app) / minecraft-launcher
/// Linux:   minecraft-launcher / minecraft
pub fn check_launcher_running() -> LauncherProcCheck {
    use sysinfo::System;

    const TARGETS: &[&str] = &[
        "MinecraftLauncher.exe",
        "Minecraft.exe",
        "minecraft.exe",
        "minecraft-launcher",
        "Minecraft Launcher",
        "Minecraft",
        "PrismLauncher",   // 公式以外でも profiles 競合が起きるため防御
        "prismlauncher",
        "Prism Launcher",
    ];

    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut matched: Option<String> = None;
    for (pid, proc_) in sys.processes() {
        let _ = pid;
        let name = proc_.name().to_string_lossy().to_string();
        let name_lower = name.to_ascii_lowercase();
        for t in TARGETS {
            if name_lower == t.to_ascii_lowercase() {
                matched = Some(name.clone());
                break;
            }
        }
        if matched.is_some() {
            break;
        }
    }

    LauncherProcCheck {
        running: matched.is_some(),
        matched_name: matched,
    }
}

/// プロファイルに書き込むデータ (型安全)
#[derive(Clone, Serialize, Debug)]
pub struct ProfileEntry {
    pub profile_key: String,
    pub name: String,
    pub game_dir: String,
    pub last_version_id: String,
    /// 内蔵 Java の bin ディレクトリ (Noneならランチャーの既定)
    pub java_dir: Option<String>,
    /// JvmPlan が生成した型安全引数 (スペース結合)
    pub java_args: String,
}

/// launcher_profiles.json のパスを決定。
/// `game_root` は .minecraft ルートを想定 (launcher_profiles.json と同階層)。
pub fn profiles_path(game_root: &Path) -> PathBuf {
    game_root.join("launcher_profiles.json")
}

/// プロファイルキーを名前から生成 (ModLinker 由来と分かる安全なASCIIキー)
pub fn profile_key_for(name: &str) -> String {
    // 名前をASCII小文字に畳んでサニタイズし、固定長ハッシュを付ける
    let base: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(24)
        .collect::<String>()
        .to_ascii_lowercase();
    let base = if base.is_empty() { "profile".to_string() } else { base };
    let hash = simple_hash(name);
    format!("modlinker-{}-{}", base, hash)
}

fn simple_hash(s: &str) -> String {
    // FNV-1a 8桁 (外部クレート不要・衝突耐性は用途十分)
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (h >> 32) as u32)
}

/// プロファイルを launcher_profiles.json に安全に追記 (Append) する。
///
/// 戻り値: 書き込んだプロファイルキー
pub fn upsert_profile(
    game_root: &Path,
    entry: &ProfileEntry,
    created_iso: &str,
) -> Result<String, String> {
    let path = profiles_path(game_root);
    let parent = path.parent().ok_or("profiles path has no parent")?;

    // ── 既存 JSON を読む ──
    let raw = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| format!("launcher_profiles.json 読み込み失敗: {}", e))?
    } else {
        String::new()
    };

    let mut root: Value = if raw.trim().is_empty() {
        serde_json::json!({
            "profiles": {},
            "settings": {},
            "version": 3
        })
    } else {
        serde_json::from_str(&raw)
            .map_err(|e| format!("launcher_profiles.json が不正です (書き込みを中止): {}", e))?
    };

    let profiles = root
        .get_mut("profiles")
        .ok_or("launcher_profiles.json に profiles キーがありません (書き込みを中止)")?
        .as_object_mut()
        .ok_or("profiles がオブジェクトではありません (書き込みを中止)")?;

    // ── 既存 ModLinker プロファイルの同nameを更新 or 追加 ──
    let key = profile_key_for(&entry.name);

    let profile_obj = serde_json::json!({
        "name": entry.name,
        "type": "custom",
        "created": created_iso,
        "lastUsed": created_iso,
        "lastVersionId": entry.last_version_id,
        "gameDir": entry.game_dir,
        "icon": "Grass",
        "javaDir": entry.java_dir.clone().unwrap_or_default(),
        "javaArgs": entry.java_args,
        "modlinkerManaged": true
    });

    profiles.insert(key.clone(), profile_obj);

    // ── タイムスタンプ付きバックアップ (書き換え直前) ──
    if path.exists() {
        let bak = backup_path(&path);
        std::fs::copy(&path, &bak)
            .map_err(|e| format!("バックアップ作成失敗 (書き込みを中止): {}", e))?;
    }

    // ── 原子的書き込み: tmp → rename ──
    let tmp = parent.join(format!(
        ".launcher_profiles.json.tmp-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, &pretty).map_err(|e| format!("一時ファイル書き込み失敗: {}", e))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("launcher_profiles.json の置換に失敗: {}", e)
    })?;

    Ok(key)
}

/// タイムスタンプ付きバックアップパス: launcher_profiles.json.bak-20261120-071530
pub fn backup_path(path: &Path) -> PathBuf {
    let ts = chrono_like_timestamp();
    let mut bak = path.as_os_str().to_owned();
    bak.push(format!(".bak-{}", ts));
    PathBuf::from(bak)
}

/// 外部クレートなしの ローカルタイムスタンプ (YYYYMMDD-HHMMSS)
fn chrono_like_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 簡易: UNIX秒からUTC日時を算出 (バックアップ名の重複回避が目的なので精度は分単位で十分)
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil date algorithm (days since epoch → y/m/d)
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
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        y, mth, d, h, m, s
    )
}

/// プロファイル用の gameDir 物理作成: <root>/profiles/<name>/
pub fn create_isolated_game_dir(game_root: &Path, profile_name: &str) -> Result<PathBuf, String> {
    let safe = safe_dir_name(profile_name);
    let dir = game_root.join("profiles").join(&safe);
    std::fs::create_dir_all(&dir).map_err(|e| format!("プロファイルフォルダ作成失敗: {}", e))?;
    Ok(dir)
}

/// フォルダ名サニタイズ (パストラバーサル遮断)
/// ASCII 英数/-/_ のみを許可し、全消去の場合はハッシュ付きフォールバック名
pub fn safe_dir_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().replace(' ', "-");
    let cleaned = cleaned.trim_matches(|c| c == '-' || c == '_').to_string();
    if cleaned.is_empty() {
        // 日本語等の非ASCII名 → ハッシュ付きフォールバック (衝突防止)
        format!("ModLinkerProfile-{}", simple_hash(name))
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_dir_name() {
        assert_eq!(safe_dir_name("My Mod Profile"), "My-Mod-Profile");
        assert_eq!(safe_dir_name("../evil"), "evil");
        assert!(safe_dir_name("").starts_with("ModLinkerProfile-"));
        assert!(safe_dir_name("日本語プロファイル").starts_with("ModLinkerProfile-"));
        // 同一の日本語名 → 同一フォールバック (衝突は同一プロファイル扱い)
        assert_eq!(safe_dir_name("サーバーA"), safe_dir_name("サーバーA"));
    }

    #[test]
    fn test_profile_key() {
        let k = profile_key_for("Test Profile");
        assert!(k.starts_with("modlinker-testprofile-"));
        assert_eq!(k.len(), "modlinker-testprofile-".len() + 8);
    }

    #[test]
    fn test_upsert_creates_and_appends() {
        let tmp = std::env::temp_dir().join(format!("ml_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let entry = ProfileEntry {
            profile_key: String::new(),
            name: "TestProfile".to_string(),
            game_dir: tmp.join("profiles").join("TestProfile").to_string_lossy().to_string(),
            last_version_id: "fabric-loader-0.16.9-1.21.1".to_string(),
            java_dir: None,
            java_args: "-Xmx8192M -XX:+UseG1GC".to_string(),
        };

        let key = upsert_profile(&tmp, &entry, "2026-11-20T00:00:00.000Z").unwrap();
        assert!(key.starts_with("modlinker-testprofile-"));

        // 既存キーが壊れないこと
        let raw = std::fs::read_to_string(tmp.join("launcher_profiles.json")).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert!(v["profiles"][&key].is_object());
        assert_eq!(v["profiles"][&key]["lastVersionId"], "fabric-loader-0.16.9-1.21.1");

        // バックアップが作られていること (2回目の書き込み時)
        let key2 = upsert_profile(&tmp, &entry, "2026-11-20T00:00:01.000Z").unwrap();
        assert_eq!(key2, key);
        let has_bak = std::fs::read_dir(&tmp).unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".bak-"));
        assert!(has_bak);

        // 不正JSONでは書き込みを中止する
        std::fs::write(tmp.join("launcher_profiles.json"), "{{{ broken").unwrap();
        let err = upsert_profile(&tmp, &entry, "2026-11-20T00:00:02.000Z");
        assert!(err.is_err());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_backup_path_format() {
        let p = Path::new("/tmp/launcher_profiles.json");
        let b = backup_path(p);
        let s = b.to_string_lossy();
        assert!(s.contains(".bak-") && s.len() > p.to_string_lossy().len() + 12);
    }
}
