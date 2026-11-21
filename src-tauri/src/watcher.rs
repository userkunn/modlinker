// ═══════════════════════════════════════════════════════════
//  ModLinker v2 – スマート手動MOD自動回収 (ハイブリッド検知)
//
//  ■ 設計 (要件 2-A)
//   1. 待機モーダルが開いている間だけダウンロードフォルダを監視する
//      (常時バックグラウンド監視は絶対禁止)。
//   2. ブラウザのダウンロード完了 (.crdownload / .part / .tmp → .jar の
//      リネーム・作成) を notify クレートでフック。
//   3. SHA-512 ハッシュを即座に計算し、マニフェスト要求ハッシュと
//      完全一致すれば目的フォルダへ fs::rename で移動し、即座に監視を
//      停止・破棄する (対象が全て揃った場合)。
//      ※ ファイル名ではなく必ずハッシュ一致のみで判定する。
//   4. ファイルロック中は数ミリ秒待機してリトライする安全なエラーハンドリング。
// ═══════════════════════════════════════════════════════════

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher, Event, EventKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

/// 監視対象ターゲット (フロントから差分結果の manual エントリを受け取る)
#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct WatchTarget {
    pub file_name: String,
    pub target_dir: String,
    pub sha512: String,
}

#[derive(Clone, Serialize, Debug)]
pub struct WatcherEvent {
    pub session_id: String,
    pub phase: String,       // "waiting" | "detected" | "hashing" | "placed" | "error" | "completed" | "stopped"
    pub message: String,
    pub matched_file: Option<String>,
    pub remaining: usize,
}

/// 監視セッション (停止フラグ + watcher + ループハンドル)
struct WatchSession {
    stop_flag: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
    loop_handle: tauri::async_runtime::JoinHandle<()>,
    watch_dir: PathBuf,
}

pub struct WatcherManager {
    sessions: std::sync::Mutex<std::collections::HashMap<String, WatchSession>>,
}

#[allow(dead_code)]
impl WatcherManager {
    pub fn new() -> Self {
        Self {
            sessions: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn insert(&self, id: String, s: WatchSession) {
        self.sessions.lock().unwrap().insert(id, s);
    }

    pub fn get_dir(&self, id: &str) -> Option<PathBuf> {
        self.sessions.lock().unwrap().get(id).map(|s| s.watch_dir.clone())
    }
}

/// ファイルロック中リトライ付きの rename (数ミリ秒待機 × N 回)
fn rename_with_retry(src: &Path, dest: &Path, attempts: u32, delay_ms: u64) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("フォルダ作成失敗: {}", e))?;
    }
    let mut last_err = String::new();
    for i in 0..attempts {
        // 移動先に既存ファイルがある場合はハッシュ一致でしかここに来ないので上書き
        if dest.exists() {
            let _ = std::fs::remove_file(dest);
        }
        match std::fs::rename(src, dest) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = e.to_string();
                // 別ファイルシステム等の恒久エラーは copy フォールバック
                if std::fs::copy(src, dest).is_ok() {
                    let _ = std::fs::remove_file(src);
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(delay_ms * (i as u64 + 1)));
            }
        }
    }
    Err(format!("ファイル移動失敗 (ロック中の可能性): {}", last_err))
}

/// ファイルを sync で SHA-512 (spawn_blocking で呼ぶ前提)
fn sha512_of(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(format!("{:x}", Sha512::digest(&data)))
}

/// ダウンロード中一時ファイル拡張子かどうか
fn is_partial(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".crdownload")
        || lower.ends_with(".part")
        || lower.ends_with(".tmp")
        || lower.ends_with(".download")
        || lower.ends_with(".opdownload")   // Opera
}

/// フォルダを軽スキャンしてターゲットハッシュに一致するファイルを探し、配置する。
/// 配置先: dest_root (= ゲームディレクトリ) / target_dir / file_name
/// 戻り値: (配置できた target の file_name 一覧, 残りターゲット)
fn scan_and_place(
    watch_dir: &Path,
    dest_root: &Path,
    targets: Vec<WatchTarget>,
    placed_log: &mut Vec<String>,
) -> (Vec<String>, Vec<WatchTarget>) {
    let mut placed = Vec::new();
    let mut pending = targets;
    let Ok(rd) = std::fs::read_dir(watch_dir) else { return (placed, pending) };

    if pending.is_empty() {
        return (placed, pending);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if is_partial(&name) {
            continue; // ダウンロード中
        }
        // .jar / .zip のみ対象 (MOD同期対象と同一)
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".jar") || lower.ends_with(".zip")) {
            continue;
        }
        candidates.push(p);
    }

    for cand in candidates {
        if pending.is_empty() {
            break;
        }
        // ハッシュ計算 → 【必ずハッシュ一致のみで判定】(ファイル名は使わない)
        let Some(hash) = sha512_of(&cand) else { continue };
        let Some(target) = pending
            .iter()
            .find(|t| {
                !t.sha512.is_empty()
                    && t.sha512.to_ascii_lowercase() == hash.to_ascii_lowercase()
                    && !placed_log.contains(&t.file_name)
            })
            .cloned()
        else {
            // どのターゲットとも一致しないファイル → 触らない
            continue;
        };

        let dest_base = target.target_dir.trim_start_matches(['/', '\\']);
        let dest = dest_root.join(dest_base).join(&target.file_name);

        match rename_with_retry(&cand, &dest, 50, 8) {
            Ok(()) => {
                placed.push(target.file_name.clone());
                placed_log.push(target.file_name.clone());
                pending.retain(|t| t.file_name != target.file_name);
            }
            Err(_) => { /* 次のスキャンでリトライされる */ }
        }
    }
    (placed, pending)
}

/// 監視を開始する (モーダルが開いている間のみ有効)。
///
/// - `watch_dir`: 監視するダウンロードフォルダ
/// - `dest_root`: 配置先のゲームディレクトリ (target_dir はこの直下に作られる)
///
/// 戻り値: session_id (stop / dir 照会に使用)
pub fn start_watcher(
    app: AppHandle,
    session_id: String,
    targets: Vec<WatchTarget>,
    watch_dir: PathBuf,
    dest_root: PathBuf,
    manager: &WatcherManager,
) -> Result<(), String> {
    if targets.is_empty() {
        return Err("監視対象がありません。".into());
    }
    if !watch_dir.exists() {
        return Err(format!("監視先フォルダが存在しません: {}", watch_dir.display()));
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = mpsc::unbounded_channel::<PathBuf>();

    // ── notify ウォッチャー (.crdownload → .jar リネーム・作成をフック) ──
    let tx_event = tx.clone();
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(ev) = res {
            // 作成 / リネーム / 書き込み完了系のイベントのみ通知
            let relevant = matches!(
                ev.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            );
            if !relevant {
                return;
            }
            for path in ev.paths {
                let _ = tx_event.send(path);
            }
        }
    })
    .map_err(|e| format!("ウォッチャー初期化失敗: {}", e))?;

    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("監視開始失敗: {}", e))?;

    let emit_event = |app: &AppHandle, ev: WatcherEvent| {
        let _ = app.emit("watcher-progress", ev);
    };

    emit_event(&app, WatcherEvent {
        session_id: session_id.clone(),
        phase: "waiting".into(),
        message: format!("📥 ブラウザでのダウンロード完了を待機中... (監視先: {})", watch_dir.display()),
        matched_file: None,
        remaining: targets.len(),
    });

    // ── 監視ループ (イベント駆動 + 2秒フォールバックスキャンのハイブリッド) ──
    let stop_flag_l = stop_flag.clone();
    let session_id_l = session_id.clone();
    let dir_l = watch_dir.clone();
    let dest_l = dest_root.clone();
    let app_l = app.clone();
    let initial_remaining = targets.len();

    let handle = tauri::async_runtime::spawn(async move {
        let mut pending = targets;
        let mut placed_log: Vec<String> = Vec::new();
        let mut tick = tokio::time::interval(Duration::from_millis(2000));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // 配置処理 (spawn_blocking で実行して結果を反映する共通ヘルパ)
        async fn run_scan(
            dir: PathBuf,
            dest: PathBuf,
            pending: Vec<WatchTarget>,
            log: Vec<String>,
        ) -> (Vec<String>, Vec<WatchTarget>, Vec<String>) {
            tokio::task::spawn_blocking(move || {
                let mut log = log;
                let (placed, remaining) = scan_and_place(&dir, &dest, pending, &mut log);
                (placed, remaining, log)
            })
            .await
            .unwrap_or_else(|_| (Vec::new(), Vec::new(), Vec::new()))
        }

        loop {
            if stop_flag_l.load(Ordering::Relaxed) {
                break;
            }

            let (do_tick, received) = tokio::select! {
                _ = tick.tick() => (true, None),
                ev = rx.recv() => (false, ev),
            };

            let trigger_name = if do_tick {
                None
            } else {
                match received {
                    Some(p) => Some(p),
                    None => break, // チャネル閉鎖
                }
            };

            // notify イベント時は対象拡張子 (.jar/.zip の完成形) のみスキャン
            if let Some(p) = &trigger_name {
                let ext_ok = p.extension().map(|e| {
                    let el = e.to_string_lossy().to_ascii_lowercase();
                    el == "jar" || el == "zip"
                }).unwrap_or(false);
                if !ext_ok {
                    continue;
                }
                emit_event(&app_l, WatcherEvent {
                    session_id: session_id_l.clone(),
                    phase: "detected".into(),
                    message: format!("🔍 検知: {} — ハッシュ検証中...", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                    matched_file: None,
                    remaining: pending.len(),
                });
            }

            let (placed, remaining, log) = run_scan(dir_l.clone(), dest_l.clone(), pending, placed_log).await;
            pending = remaining;
            placed_log = log;

            if !placed.is_empty() {
                for f in &placed {
                    emit_event(&app_l, WatcherEvent {
                        session_id: session_id_l.clone(),
                        phase: "placed".into(),
                        message: format!("✔ 配置完了: {} (SHA-512 ハッシュ照合済み)", f),
                        matched_file: Some(f.clone()),
                        remaining: pending.len(),
                    });
                }
            }

            if pending.is_empty() && !placed_log.is_empty() {
                emit_event(&app_l, WatcherEvent {
                    session_id: session_id_l.clone(),
                    phase: "completed".into(),
                    message: "🎉 すべての未登録MODが自動回収されました。監視を停止します。".into(),
                    matched_file: None,
                    remaining: 0,
                });
                break;
            }
            let _ = initial_remaining;
        }
    });

    manager.insert(session_id, WatchSession {
        stop_flag,
        _watcher: Some(watcher),
        loop_handle: handle,
        watch_dir,
    });

    Ok(())
}

/// 監視を確実に破棄して停止する (キャンセル時・完了時)。
pub fn stop_watcher(session_id: &str, manager: &WatcherManager) -> Result<(), String> {
    let session = manager.sessions.lock().unwrap().remove(session_id);
    match session {
        Some(s) => {
            s.stop_flag.store(true, Ordering::Relaxed);
            // watcher をドロップして OS 監視を即解除
            drop(s._watcher);
            let _ = s.loop_handle.abort(); // select ループを即時終了
            Ok(())
        }
        None => Err("指定セッションが見つかりません (すでに停止済みの可能性)".into()),
    }
}

/// ワンショット一括スキャン (Firefox/Tor 等の監視不可環境向けフォールバック)
#[derive(Serialize, Debug)]
pub struct ScanResult {
    pub matched: Vec<WatchTarget>,
    pub missed: Vec<String>,
}

pub async fn scan_once(
    watch_dir: PathBuf,
    dest_root: PathBuf,
    targets: Vec<WatchTarget>,
) -> Result<ScanResult, String> {
    if !watch_dir.exists() {
        return Err(format!("フォルダが存在しません: {}", watch_dir.display()));
    }
    tokio::task::spawn_blocking(move || {
        let mut placed_log: Vec<String> = Vec::new();
        let (placed, remaining) = scan_and_place(&watch_dir, &dest_root, targets.clone(), &mut placed_log);
        let matched = targets
            .iter()
            .filter(|t| placed.contains(&t.file_name))
            .cloned()
            .collect();
        let missed = remaining.iter().map(|t| t.file_name.clone()).collect();
        Ok(ScanResult { matched, missed })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 手動選択 / ドラッグ&ドロップされたファイルを配置する (複数一括対応)
#[derive(Serialize, Debug)]
pub struct PlaceResult {
    pub placed: Vec<WatchTarget>,
    pub failed: Vec<(String, String)>, // (path, reason)
}

pub async fn place_files(paths: Vec<PathBuf>, targets: Vec<WatchTarget>, dest_root: PathBuf) -> Result<PlaceResult, String> {
    tokio::task::spawn_blocking(move || {
        let mut pending = targets;
        let mut placed = Vec::new();
        let mut failed = Vec::new();

        for p in paths {
            if !p.is_file() {
                failed.push((p.to_string_lossy().to_string(), "ファイルが存在しません".into()));
                continue;
            }
            let Some(hash) = sha512_of(&p) else {
                failed.push((p.to_string_lossy().to_string(), "ハッシュ計算失敗".into()));
                continue;
            };
            let Some(t) = pending
                .iter()
                .find(|t| !t.sha512.is_empty() && t.sha512.to_ascii_lowercase() == hash.to_ascii_lowercase())
                .cloned()
            else {
                failed.push((
                    p.to_string_lossy().to_string(),
                    "ハッシュがどのマニフェスト対象とも一致しません".into(),
                ));
                continue;
            };

            let dest_base = t.target_dir.trim_start_matches(['/', '\\']);
            let dest = dest_root.join(dest_base).join(&t.file_name);
            match rename_with_retry(&p, &dest, 50, 8) {
                Ok(()) => {
                    pending.retain(|x| x.file_name != t.file_name);
                    placed.push(t);
                }
                Err(e) => failed.push((p.to_string_lossy().to_string(), e)),
            }
        }
        Ok(PlaceResult { placed, failed })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_target(name: &str, dir: &str, content: &[u8]) -> WatchTarget {
        WatchTarget {
            file_name: name.to_string(),
            target_dir: dir.to_string(),
            sha512: format!("{:x}", Sha512::digest(content)),
        }
    }

    #[test]
    fn test_scan_and_place_hash_match() {
        let tmp = std::env::temp_dir().join(format!("ml_watch_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let dl_dir = tmp.join("Downloads");
        let game = tmp.join(".minecraft");
        std::fs::create_dir_all(&dl_dir).unwrap();
        std::fs::create_dir_all(&game).unwrap();

        let content_a = b"fake jar content A";
        let content_b = b"another jar B";

        // ダウンロードフォルダに jar を配置 (ブラウザDL完了を模擬)
        std::fs::write(dl_dir.join("some-random-name-1.0.jar"), content_a).unwrap();
        std::fs::write(dl_dir.join("totally-different-name.jar"), content_b).unwrap();
        // 一致しないファイル (触られてはいけない)
        std::fs::write(dl_dir.join("unrelated.jar"), b"no match here").unwrap();
        // ダウンロード中ファイル (スキップされるべき)
        std::fs::write(dl_dir.join("downloading.jar.crdownload"), content_a).unwrap();

        let targets = vec![
            make_target("mod-a.jar", "mods", content_a),
            make_target("mod-b.jar", "mods", content_b),
        ];

        let mut log = Vec::new();
        let (placed, remaining) = scan_and_place(&dl_dir, &game, targets, &mut log);

        // ハッシュ一致のみで判定 → ランダムなファイル名でも正しく配置される
        assert_eq!(placed.len(), 2, "placed: {:?}", placed);
        assert!(game.join("mods/mod-a.jar").exists());
        assert!(game.join("mods/mod-b.jar").exists());
        // 内容が一致
        assert_eq!(std::fs::read(game.join("mods/mod-a.jar")).unwrap(), content_a);
        // 未一致ファイルは無傷
        assert!(dl_dir.join("unrelated.jar").exists());
        // 残りは 0
        assert_eq!(remaining.len(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_scan_and_place_ignores_partial_files() {
        let tmp = std::env::temp_dir().join(format!("ml_watch_p_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let dl_dir = tmp.join("Downloads");
        let game = tmp.join(".minecraft");
        std::fs::create_dir_all(&dl_dir).unwrap();
        std::fs::create_dir_all(&game).unwrap();

        let content = b"partial content";
        // .part / .tmp は拡張子が jar でないのでスキップ (ブラウザがリネームするまで待つ)
        std::fs::write(dl_dir.join("file.jar.part"), content).unwrap();
        std::fs::write(dl_dir.join("file.tmp"), content).unwrap();

        let targets = vec![make_target("mod.jar", "mods", content)];
        let mut log = Vec::new();
        let (placed, _) = scan_and_place(&dl_dir, &game, targets, &mut log);
        assert_eq!(placed.len(), 0);
        assert!(!game.join("mods/mod.jar").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_place_files_multiple() {
        let tmp = std::env::temp_dir().join(format!("ml_place_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let game = tmp.join(".minecraft");
        std::fs::create_dir_all(&game).unwrap();

        let content = b"dragged jar";
        let src = tmp.join("picked-file.jar");
        std::fs::write(&src, content).unwrap();

        let targets = vec![make_target("placed-mod.jar", "shaderpacks", content)];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(place_files(vec![src.clone()], targets, game.clone())).unwrap();

        assert_eq!(result.placed.len(), 1);
        assert!(game.join("shaderpacks/placed-mod.jar").exists());
        assert!(!src.exists(), "元ファイルは移動(消滅)しているべき");
        assert_eq!(result.failed.len(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_place_files_hash_mismatch_rejected() {
        let tmp = std::env::temp_dir().join(format!("ml_place_x_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let game = tmp.join(".minecraft");
        std::fs::create_dir_all(&game).unwrap();

        let src = tmp.join("bad.jar");
        std::fs::write(&src, b"malicious or wrong file").unwrap();

        let targets = vec![make_target("mod.jar", "mods", b"expected content")];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(place_files(vec![src.clone()], targets, game.clone())).unwrap();

        assert_eq!(result.placed.len(), 0);
        assert_eq!(result.failed.len(), 1);
        // ハッシュ不一致のファイルは元の場所に残る (削除されない)
        assert!(src.exists());
        assert!(!game.join("mods/mod.jar").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_is_partial() {
        assert!(is_partial("file.jar.crdownload"));
        assert!(is_partial("file.jar.part"));
        assert!(is_partial("FILE.TMP"));
        assert!(!is_partial("file.jar"));
    }
}
