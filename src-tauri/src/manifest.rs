// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – Manifest (v2 format + v1 backward compat)
//
//  【型安全ポリシー】
//  マニフェストから「生のJVM引数文字列」は絶対に受け取らない。
//  ローダー/JVMチューニングはすべて enum/数値の構造体で検証してから
//  ホワイトリスト型ビルダー (launcher/jvm.rs) に渡す。
// ═══════════════════════════════════════════════════════════

use serde::{Deserialize, Serialize};

/// v1 の MOD エントリ (旧形式のまま・後方互換)
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ModEntry {
    pub file_name: String,
    pub target_dir: String,
    pub sha512: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default)]
    pub version_number: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub matched: bool,
}

/// MODローダーの種別 (文字列フリーの列挙型 — インジェクション構造的遮断)
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Vanilla,
    Fabric,
    Forge,
    NeoForge,
}

impl LoaderKind {
    #[allow(dead_code)]
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "vanilla" | "none" | "" => Some(LoaderKind::Vanilla),
            "fabric" => Some(LoaderKind::Fabric),
            "forge" => Some(LoaderKind::Forge),
            "neoforge" => Some(LoaderKind::NeoForge),
            _ => None,
        }
    }
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            LoaderKind::Vanilla => "vanilla",
            LoaderKind::Fabric => "fabric",
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
        }
    }
}

/// マニフェストに埋め込むローダー情報
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct LoaderInfo {
    pub kind: LoaderKind,
    /// Minecraft バージョン (例: "1.21.1") — 形式検証済み
    pub mc_version: String,
    /// ローダー版 (None なら自動解決)
    #[serde(default)]
    pub loader_version: Option<String>,
}

/// GC ポリシー (自由文字列は禁止・enum のみ)
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GcPolicy {
    /// マニフェスト推奨 (Java21 なら ZGC、それ以外は G1 に自動降格)
    #[default]
    Auto,
    Zgc,
    G1,
}

/// 【完全型安全】JVM チューニング構造体。
/// この構造体以外の形式 (生文字列の引数リスト等) は serde レベルで拒否される。
/// 例: { "recommended_java": 21, "gc": "zgc", "memory_mb": 10240, "always_pre_touch": true }
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct JvmTuning {
    /// 推奨 Java メジャーバージョン (8..=32)
    #[serde(default = "default_recommended_java")]
    pub recommended_java: u8,
    /// GC 方針
    #[serde(default)]
    pub gc: GcPolicy,
    /// 要求ヒープメモリ (MB)
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,
    /// AlwaysPreTouch を付けるか
    #[serde(default)]
    pub always_pre_touch: bool,
}

fn default_recommended_java() -> u8 { 21 }
fn default_memory_mb() -> u32 { 4096 }

impl Default for JvmTuning {
    fn default() -> Self {
        Self {
            recommended_java: default_recommended_java(),
            gc: GcPolicy::Auto,
            memory_mb: default_memory_mb(),
            always_pre_touch: false,
        }
    }
}

impl JvmTuning {
    /// 不正値のクランプ (構造体レベルのサニタイズ)
    pub fn sanitized(&self) -> Self {
        let mut t = self.clone();
        t.recommended_java = t.recommended_java.clamp(8, 32);
        t.memory_mb = t.memory_mb.clamp(1024, 65_536);
        t
    }
}

/// マニフェスト v2 ルート構造
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct ManifestV2 {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub loader: Option<LoaderInfo>,
    #[serde(default)]
    pub tuning: Option<JvmTuning>,
    #[serde(default)]
    pub mods: Vec<ModEntry>,
}

fn default_format_version() -> u32 { 2 }

impl ManifestV2 {
    /// v1 (配列) / v2 (オブジェクト) 両形式を解析。
    /// 不明なキーは無視、v1 では loader/tuning は None。
    pub fn parse(json: &str) -> Result<Self, String> {
        let trimmed = json.trim();
        if trimmed.is_empty() {
            return Err("マニフェストが空です。".into());
        }

        // v2 オブジェクト形式を試す
        if let Ok(v2) = serde_json::from_str::<ManifestV2>(trimmed) {
            if v2.format_version >= 2 || v2.loader.is_some() || v2.tuning.is_some() || !v2.mods.is_empty() {
                return Ok(v2);
            }
        }

        // v1 配列形式 (後方互換)
        let mods: Vec<ModEntry> = serde_json::from_str(trimmed)
            .map_err(|e| format!("マニフェストJSONの解析に失敗: {}", e))?;
        Ok(ManifestV2 {
            format_version: 1,
            created_by: None,
            loader: None,
            tuning: None,
            mods,
        })
    }

    /// MC バージョン文字列の形式検証 (X.Y.Z / X.Y / X.Y.Z-W.pre 等の安全な形式のみ許可)
    /// インストーラー引数インジェクション防止のため、許可文字は数字と '.' '-' '_' '+' のみ。
    pub fn validate_version(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 32
            && s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '_' || c == '+')
            && s.chars().next().map_or(false, |c| c.is_ascii_digit())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_v1_array_format() {
        let v1 = r#"[
            {"file_name": "a.jar", "target_dir": "mods", "sha512": "abc", "source": "modrinth", "matched": true}
        ]"#;
        let m = ManifestV2::parse(v1).unwrap();
        assert_eq!(m.format_version, 1);
        assert_eq!(m.mods.len(), 1);
        assert!(m.loader.is_none());
        assert!(m.tuning.is_none());
    }

    #[test]
    fn test_parse_v2_object_format() {
        let v2 = r#"{
            "format_version": 2,
            "loader": {"kind": "fabric", "mc_version": "1.21.1", "loader_version": null},
            "tuning": {"recommended_java": 21, "gc": "zgc", "memory_mb": 10240, "always_pre_touch": true},
            "mods": []
        }"#;
        let m = ManifestV2::parse(v2).unwrap();
        assert_eq!(m.format_version, 2);
        assert!(m.loader.is_some());
        assert_eq!(m.loader.unwrap().kind, LoaderKind::Fabric);
        let t = m.tuning.unwrap();
        assert_eq!(t.memory_mb, 10240);
        assert_eq!(t.gc, GcPolicy::Zgc);
        assert!(t.always_pre_touch);
    }

    #[test]
    fn test_parse_rejects_garbage() {
        assert!(ManifestV2::parse("{{{").is_err());
        assert!(ManifestV2::parse("").is_err());
    }

    #[test]
    fn test_version_validation_blocks_injection() {
        assert!(ManifestV2::validate_version("1.21.1"));
        assert!(ManifestV2::validate_version("1.20.4"));
        assert!(!ManifestV2::validate_version("1.21.1 -exec evil"));
        assert!(!ManifestV2::validate_version("1.21;rm -rf"));
        assert!(!ManifestV2::validate_version(""));
        assert!(!ManifestV2::validate_version("$(dangerous)"));
    }

    #[test]
    fn test_tuning_rejects_raw_jvm_args() {
        // 生文字列の jvm_args は存在しない構造として無視される (= 構造的遮断)
        let json = r#"{"format_version": 2, "jvm_args": "-XX:EVIL", "mods": []}"#;
        let m = ManifestV2::parse(json).unwrap();
        assert!(m.tuning.is_none());
    }
}
