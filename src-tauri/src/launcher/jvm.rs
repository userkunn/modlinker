// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – 【完全型安全】JVM / ZGC 最適化ビルダー
//
//  ■ 設計原則 (要件 1-D)
//   1. マニフェストJSONから「生のJVM引数文字列」を絶対に受け取らない。
//      → JvmTuning 構造体 (enum + 数値) のみを解釈する。
//      → コマンド / 引数インジェクションを100%構造的に遮断する。
//   2. 生成される引数は「このファイル内のホワイトリスト定数」と
//      「数値検証済みの -Xmx{N}M」のみ。
//   3. バージョン・環境分岐:
//      - Java 21 + ZGC 推奨 → 世代別 ZGC (-XX:+UseZGC -XX:+ZGenerational)、上限16GB
//      - Java 17 / 互換フォールバック → G1GC (-XX:+UseG1GC)、上限8GB
// ═══════════════════════════════════════════════════════════

use serde::{Deserialize, Serialize};

use crate::manifest::{GcPolicy, JvmTuning};

/// 適用された GC の種別
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppliedGc {
    ZgcGenerational,
    Zgc,
    G1,
}

/// JVM 引数プラン (生成結果)
#[derive(Clone, Serialize, Debug)]
pub struct JvmPlan {
    /// 生成された JVM 引数 (ホワイトリスト定数 + 検証済み数値のみ)
    pub args: Vec<String>,
    /// 適用された GC
    pub gc_applied: AppliedGc,
    /// クランプ後のヒープ上限 (MB)
    pub memory_mb: u32,
    /// 推奨 Java メジャーバージョン
    pub recommended_java: u8,
    /// 降格・調整の理由 (UI表示用)
    pub notes: Vec<String>,
}

// ── 上限定数 ─────────────────────────────────────────────
/// 世代別 ZGC 時のヒープ上限 (要件: 最大16GBまで解放)
const ZGC_MAX_MEMORY_MB: u32 = 16 * 1024;
/// G1GC 時のヒープ上限 (要件: 8GBキャップ)
const G1_MAX_MEMORY_MB: u32 = 8 * 1024;
/// 最低保証ヒープ
const MIN_MEMORY_MB: u32 = 1024;
/// 物理RAMに対する安全マージン (OS + マイクラ本体分, MB)
const PHYSICAL_RAM_RESERVE_MB: u32 = 2048;

// ── ホワイトリスト定数 (ここ以外で引数を生成しない) ──────
const ARGS_ZGC_GENERATIONAL: &[&str] = &[
    "-XX:+UseZGC",
    "-XX:+ZGenerational",
    "-XX:+DisableExplicitGC",
];
const ARGS_G1: &[&str] = &[
    "-XX:+UseG1GC",
    "-XX:+DisableExplicitGC",
    "-XX:G1NewSizePercent=20",
    "-XX:G1ReservePercent=20",
    "-XX:MaxGCPauseMillis=50",
];
const ARG_ALWAYS_PRE_TOUCH: &str = "-XX:+AlwaysPreTouch";
const ARG_UTF8: &str = "-Dfile.encoding=UTF-8";

/// 数値を安全に -Xmx{N}M に整形 (N は u32 検証済みのみ通る)
fn format_xmx(mb: u32) -> String {
    // mb は u32 なので文字列としても安全
    format!("-Xmx{}M", mb)
}

/// 物理RAM (MB) を取得 (取得失敗時は None)
pub fn physical_ram_mb() -> Option<u32> {
    use sysinfo::System;
    let sys = System::new_all();
    let bytes = sys.total_memory();
    if bytes == 0 {
        return None;
    }
    Some((bytes / (1024 * 1024)) as u32)
}

/// 【核心ロジック】JvmTuning 構造体から JVM プランを生成。
///
/// 引数:
/// - `tuning`: マニフェスト由来の型安全チューニング (生文字列は存在しない)
/// - `java_major`: 実際に使用する Java のメジャーバージョン (探索結果)
/// - `zgc_enabled`: 参加者側UIの「🚀 ZGC最適化を有効にする」トグル
/// - `ram_mb`: 物理メモリ (None 可)
pub fn build_jvm_plan(
    tuning: &JvmTuning,
    java_major: u8,
    zgc_enabled: bool,
    ram_mb: Option<u32>,
) -> JvmPlan {
    let tuning = tuning.sanitized();
    let mut notes: Vec<String> = Vec::new();

    // ── 1. GC 選択 (バージョン・環境分岐とセーフガード) ──
    //      Java 21 + ZGC要求 → 世代別ZGC
    //      Java 17 以下 / フォールバック → G1GC (安定)
    let wants_zgc = zgc_enabled && matches!(tuning.gc, GcPolicy::Zgc | GcPolicy::Auto);
    let (gc_applied, gc_args, max_cap) = if wants_zgc && java_major >= 21 {
        (AppliedGc::ZgcGenerational, ARGS_ZGC_GENERATIONAL.to_vec(), ZGC_MAX_MEMORY_MB)
    } else if wants_zgc && java_major < 21 && matches!(tuning.gc, GcPolicy::Zgc) {
        // マニフェストは ZGC 推奨だが Java が古い → 安全のため G1 に自動フォールバック
        notes.push(format!(
            "ZGC requires Java 21+ (detected Java {}), falling back to G1GC",
            java_major
        ));
        (AppliedGc::G1, ARGS_G1.to_vec(), G1_MAX_MEMORY_MB)
    } else if !wants_zgc && matches!(tuning.gc, GcPolicy::Zgc) && !zgc_enabled {
        notes.push("ZGC optimization toggle is OFF, using G1GC".to_string());
        (AppliedGc::G1, ARGS_G1.to_vec(), G1_MAX_MEMORY_MB)
    } else {
        (AppliedGc::G1, ARGS_G1.to_vec(), G1_MAX_MEMORY_MB)
    };

    // ── 2. メモリクランプ ──
    let mut mem = tuning.memory_mb.max(MIN_MEMORY_MB).min(max_cap);
    if let Some(ram) = ram_mb {
        let usable = ram.saturating_sub(PHYSICAL_RAM_RESERVE_MB).max(MIN_MEMORY_MB);
        if mem > usable {
            notes.push(format!(
                "Heap clamped to {}MB (detected physical RAM {}MB)",
                usable, ram
            ));
            mem = usable.min(max_cap);
        }
    }

    // ── 3. 引数生成 (ホワイトリスト + 数値のみ) ──
    let mut args: Vec<String> = Vec::with_capacity(gc_args.len() + 4);
    args.push(format_xmx(mem)); // 数値検証済みの唯一の動的引数
    args.extend(gc_args.iter().map(|s| s.to_string()));
    if tuning.always_pre_touch {
        args.push(ARG_ALWAYS_PRE_TOUCH.to_string());
    }
    args.push(ARG_UTF8.to_string());

    JvmPlan {
        args,
        gc_applied,
        memory_mb: mem,
        recommended_java: tuning.recommended_java,
        notes,
    }
}

/// 起動失敗時の「公式 Java への自動フォールバック」用に、
/// 同一チューニングで G1GC 版 (安全側) のプランを再生成する。
pub fn fallback_g1_plan(tuning: &JvmTuning, java_major: u8, ram_mb: Option<u32>) -> JvmPlan {
    let safe = JvmTuning {
        gc: GcPolicy::G1,
        ..tuning.clone()
    };
    let mut plan = build_jvm_plan(&safe, java_major, false, ram_mb);
    plan.notes
        .push("Fallback plan (G1GC) generated after launch failure with optimized GC".to_string());
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zgc_on_java21_generational() {
        let tuning = JvmTuning {
            recommended_java: 21,
            gc: GcPolicy::Zgc,
            memory_mb: 10240,
            always_pre_touch: true,
        };
        let plan = build_jvm_plan(&tuning, 21, true, Some(32768));
        assert_eq!(plan.gc_applied, AppliedGc::ZgcGenerational);
        assert!(plan.args.contains(&"-XX:+ZGenerational".to_string()));
        assert!(plan.args.contains(&"-Xmx10240M".to_string()));
        assert!(plan.args.contains(&"-XX:+AlwaysPreTouch".to_string()));
    }

    #[test]
    fn zgc_on_java17_falls_back_to_g1() {
        let tuning = JvmTuning {
            recommended_java: 21,
            gc: GcPolicy::Zgc,
            memory_mb: 10240,
            always_pre_touch: false,
        };
        let plan = build_jvm_plan(&tuning, 17, true, None);
        assert_eq!(plan.gc_applied, AppliedGc::G1);
        assert!(plan.args.contains(&"-XX:+UseG1GC".to_string()));
        assert!(!plan.args.iter().any(|a| a.contains("ZGC")));
        // 8GBキャップ
        assert!(plan.args.contains(&"-Xmx8192M".to_string()));
        assert!(!plan.notes.is_empty());
    }

    #[test]
    fn g1_capped_8gb_zgc_capped_16gb() {
        let tuning = JvmTuning {
            recommended_java: 21,
            gc: GcPolicy::Zgc,
            memory_mb: 32768,
            always_pre_touch: false,
        };
        let p1 = build_jvm_plan(&tuning, 21, true, None);
        assert!(p1.args.contains(&"-Xmx16384M".to_string()));
        let p2 = build_jvm_plan(&tuning, 17, false, None);
        assert!(p2.args.contains(&"-Xmx8192M".to_string()));
    }

    #[test]
    fn no_injection_possible() {
        // 生文字列を受け取らない構造になっているため、
        // 数値フィールドに不正な値は serde レベルで入らない。
        // さらにメモリは u32 クランプ済み → -Xmx は必ず数字のみ。
        let tuning = JvmTuning {
            recommended_java: 21,
            gc: GcPolicy::Zgc,
            memory_mb: 4096,
            always_pre_touch: false,
        };
        let plan = build_jvm_plan(&tuning, 21, true, None);
        for a in &plan.args {
            if a.starts_with("-Xmx") {
                let num = &a[4..a.len() - 1];
                assert!(num.chars().all(|c| c.is_ascii_digit()));
                assert!(a.ends_with('M'));
            }
        }
    }

    #[test]
    fn physical_ram_respected() {
        let tuning = JvmTuning {
            recommended_java: 21,
            gc: GcPolicy::Zgc,
            memory_mb: 16384,
            always_pre_touch: false,
        };
        // RAM 4GB 環境では 2048MB までクランプされる
        let plan = build_jvm_plan(&tuning, 21, true, Some(4096));
        assert!(plan.args.contains(&"-Xmx2048M".to_string()));
    }
}
