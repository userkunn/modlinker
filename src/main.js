// ═══════════════════════════════════════════════════════════
//  ModLinker – Frontend Logic (Vanilla JS, Tauri v2 API)
// ═══════════════════════════════════════════════════════════
window.addEventListener("error", (e) => {
  const title = document.getElementById("header-title-text");
  if (title) title.textContent = "Error: " + e.message;
  fetch('http://localhost:9999', { method: 'POST', body: "ERROR: " + e.message + "\nFile: " + e.filename + "\nLine: " + e.lineno + "\nCol: " + e.colno + "\nStack: " + (e.error && e.error.stack) }).catch(err => console.error(err));
});
window.addEventListener("unhandledrejection", (e) => {
  const title = document.getElementById("header-title-text");
  if (title) title.textContent = "Rejection: " + e.reason;
  fetch('http://localhost:9999', { method: 'POST', body: "REJECTION: " + e.reason }).catch(err => console.error(err));
});
const invoke = window.__TAURI__?.core?.invoke;
const listen = window.__TAURI__?.event?.listen;
const open = window.__TAURI__?.dialog?.open;
const readTextFile = window.__TAURI__?.fs?.readTextFile;

// ── DOM要素 ──
let gamePathInput, browseBtn;

// モード切替
let navModeSync, navModeGenerate, navSettingsBtn;
let panelSync, panelGenerate, panelSettings;
let headerTitleText;

// テーマ
let themeToggleBtn, themeIcon, themeText;

// 設定
let cfApiKeyInput, cfKeySaveBtn, cfKeyClearBtn;
let r2AccountIdInput, r2BucketInput, r2AccessKeyInput, r2SecretKeyInput, r2PublicUrlInput;
let r2SaveBtn, r2ClearBtn;

// ダウンロード用
let manifestInput, skipR2Checkbox, startDiffBtn, browseManifestBtn;
let diffSection, diffTbody, diffStats, executeDownloadBtn;
let manualDlSection, manualDlList;
let currentDiffTargets = [];
let currentManualTargets = [];

// 生成用
let startGenerateBtn, saveManifestBtn;

// 共通
let progressBar, progressLabel, progressPercent;
let logConsoleSync, logConsoleGenerate, clearLogBtnSync, clearLogBtnGenerate;
let resultsTbody, resultsSection, resultsStats, modCountBadge;
let statusIndicator, exportManifestBtn;

// ── 状態 ──
let isProcessing = false;
let currentMode = 'sync'; // 'sync' | 'generate'
let lastManifest = [];
let loadedManifestContent = "";

// ── 初期化 ──
window.addEventListener("DOMContentLoaded", async () => {
  gamePathInput = document.getElementById("game-path-input");
  browseBtn = document.getElementById("browse-btn");

  navModeSync = document.getElementById("nav-mode-sync");
  navModeGenerate = document.getElementById("nav-mode-generate");
  navSettingsBtn = document.getElementById("nav-settings-btn");
  panelSync = document.getElementById("panel-sync");
  panelGenerate = document.getElementById("panel-generate");
  panelSettings = document.getElementById("panel-settings");
  headerTitleText = document.getElementById("header-title-text");

  cfApiKeyInput = document.getElementById("cf-api-key-input");
  cfKeySaveBtn = document.getElementById("cf-key-save-btn");
  cfKeyClearBtn = document.getElementById("cf-key-clear-btn");

  r2AccountIdInput = document.getElementById("r2-account-id");
  r2BucketInput = document.getElementById("r2-bucket");
  r2AccessKeyInput = document.getElementById("r2-access-key");
  r2SecretKeyInput = document.getElementById("r2-secret-key");
  r2PublicUrlInput = document.getElementById("r2-public-url");
  r2SaveBtn = document.getElementById("r2-save-btn");
  r2ClearBtn = document.getElementById("r2-clear-btn");

  manifestInput = document.getElementById("manifest-input");
  skipR2Checkbox = document.getElementById("skip-r2-checkbox");
  browseManifestBtn = document.getElementById("browse-manifest-btn");
  startDiffBtn = document.getElementById("start-diff-btn");
  diffSection = document.getElementById("diff-section");
  diffTbody = document.getElementById("diff-tbody");
  diffStats = document.getElementById("diff-stats");
  executeDownloadBtn = document.getElementById("execute-download-btn");
  manualDlSection = document.getElementById("manual-dl-section");
  manualDlList = document.getElementById("manual-dl-list");

  startGenerateBtn = document.getElementById("start-generate-btn");
  saveManifestBtn = document.getElementById("save-manifest-btn");

  progressBar = document.getElementById("progress-bar");
  progressLabel = document.getElementById("progress-label");
  progressPercent = document.getElementById("progress-percent");
  logConsoleSync = document.getElementById("log-console-sync");
  logConsoleGenerate = document.getElementById("log-console-generate");
  clearLogBtnSync = document.getElementById("clear-log-btn-sync");
  clearLogBtnGenerate = document.getElementById("clear-log-btn-generate");

  resultsTbody = document.getElementById("results-tbody");
  resultsSection = document.getElementById("results-section");
  resultsStats = document.getElementById("results-stats");
  modCountBadge = document.getElementById("mod-count-badge");
  statusIndicator = document.getElementById("status-indicator");
  exportManifestBtn = document.getElementById("export-manifest-btn");

  themeToggleBtn = document.getElementById("theme-toggle-btn");
  themeIcon = document.getElementById("theme-icon");
  themeText = document.getElementById("theme-text");

    // i18n & Custom Select Init
  if (window.i18n) {
    const container = document.getElementById("locale-custom-select-container");
    const initialLocale = window.i18n.detectLocale();
    
    if (container) {
      // Build custom select DOM
      container.innerHTML = `
        <div class="custom-select-wrapper">
          <div class="custom-select-trigger" id="custom-locale-trigger">
            <span id="custom-locale-selected-text"></span>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#a1a1aa" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="6 9 12 15 18 9"></polyline>
            </svg>
          </div>
          <div class="custom-select-options" id="custom-locale-options"></div>
        </div>
      `;

      const trigger = document.getElementById("custom-locale-trigger");
      const optionsContainer = document.getElementById("custom-locale-options");
      const selectedText = document.getElementById("custom-locale-selected-text");

      // Populate options
      window.i18n.SUPPORTED_LOCALES.forEach(loc => {
        const optDiv = document.createElement("div");
        optDiv.className = "custom-option";
        if (loc.code === initialLocale) optDiv.classList.add("selected");
        optDiv.textContent = `${loc.flag} ${loc.name}`;
        optDiv.dataset.value = loc.code;
        
        optDiv.addEventListener("click", async () => {
          // close dropdown
          optionsContainer.classList.remove("open");
          trigger.classList.remove("open");
          
          // update UI
          document.querySelectorAll(".custom-option").forEach(el => el.classList.remove("selected"));
          optDiv.classList.add("selected");
          selectedText.textContent = `${loc.flag} ${loc.name}`;
          
          // Apply locale
          await window.i18n.setLocale(loc.code);
          updateDynamicTexts();
        });
        
        optionsContainer.appendChild(optDiv);
      });

      // Set initial text
      const initLoc = window.i18n.SUPPORTED_LOCALES.find(l => l.code === initialLocale) || window.i18n.SUPPORTED_LOCALES[0];
      selectedText.textContent = `${initLoc.flag} ${initLoc.name}`;

      // Toggle dropdown
      trigger.addEventListener("click", (e) => {
        e.stopPropagation();
        optionsContainer.classList.toggle("open");
        trigger.classList.toggle("open");
      });

      // Close when clicking outside
      document.addEventListener("click", () => {
        optionsContainer.classList.remove("open");
        trigger.classList.remove("open");
      });
    }

    await window.i18n.setLocale(initialLocale);
  }

  // 動的テキストの更新用（モード切替時などに再適用する場合）
  function updateDynamicTexts() {
    updateThemeUI(document.documentElement.classList.contains("theme-light"));
    if (executeDownloadBtn) {
      const dlCount = currentDiffTargets ? currentDiffTargets.filter(d => d.action === "download").length : 0;
      const manualCount = typeof currentManualTargets !== 'undefined' ? currentManualTargets.length : 0;
      
      if (dlCount > 0) {
        executeDownloadBtn.textContent = window.i18n ? window.i18n.t("sync.download_start", [dlCount]) : `ダウンロード開始 (${dlCount} 個)`;
      } else if (manualCount > 0) {
        executeDownloadBtn.textContent = window.i18n ? window.i18n.t("sync.manual_open", [manualCount]) || `手動DLページを開く (${manualCount} 個)` : `手動DLページを開く (${manualCount} 個)`;
      } else {
        executeDownloadBtn.textContent = window.i18n ? window.i18n.t("sync.no_download") : "ダウンロード不要";
      }
    }
  }

  // ── テーマ初期化（最優先） ──
  const savedTheme = localStorage.getItem("theme");
  const isLight = savedTheme ? (savedTheme === "light") : true;
  if (isLight) {
    document.documentElement.classList.add("theme-light");
  } else {
    document.documentElement.classList.remove("theme-light");
  }
  updateThemeUI(isLight);

  // ── イベントリスナー登録（nullガード付き） ──
  if (navModeSync) navModeSync.addEventListener("click", () => switchMode("sync"));
  if (navModeGenerate) navModeGenerate.addEventListener("click", () => switchMode("generate"));
  if (navSettingsBtn) navSettingsBtn.addEventListener("click", () => switchMode("settings"));
  const navAboutBtn = document.getElementById("nav-about-btn");
  if (navAboutBtn) navAboutBtn.addEventListener("click", () => switchMode("docs"));

  if (browseBtn) browseBtn.addEventListener("click", handleBrowse);
  if (browseManifestBtn) browseManifestBtn.addEventListener("click", handleBrowseManifest);
  if (manifestInput) manifestInput.addEventListener("click", handleBrowseManifest);
  if (clearLogBtnSync) clearLogBtnSync.addEventListener("click", () => clearLog("sync"));
  if (clearLogBtnGenerate) clearLogBtnGenerate.addEventListener("click", () => clearLog("generate"));
  if (exportManifestBtn) exportManifestBtn.addEventListener("click", handleExportManifest);
  if (saveManifestBtn) saveManifestBtn.addEventListener("click", handleSaveManifest);

  if (startGenerateBtn) startGenerateBtn.addEventListener("click", handleGenerateManifest);
  if (startDiffBtn) startDiffBtn.addEventListener("click", handleCheckDiff);
  if (executeDownloadBtn) executeDownloadBtn.addEventListener("click", handleExecuteDownload);
  if (themeToggleBtn) themeToggleBtn.addEventListener("click", toggleTheme);
  if (cfKeySaveBtn) cfKeySaveBtn.addEventListener("click", saveCfApiKey);
  if (cfKeyClearBtn) cfKeyClearBtn.addEventListener("click", clearCfApiKey);

  if (r2SaveBtn) r2SaveBtn.addEventListener("click", saveR2Config);
  if (r2ClearBtn) r2ClearBtn.addEventListener("click", clearR2Config);

  // GSAP 初期アニメーションは一旦無効化

  setupProgressListener();

  // CF APIキーの読み込み
  const savedKey = localStorage.getItem("cf_api_key");
  if (savedKey && cfApiKeyInput) cfApiKeyInput.value = savedKey;

  // R2設定の読み込み（Rust側から安全に読む）
  try {
    const r2Config = await invoke("load_r2_config");
    if (r2Config) {
      if (r2AccountIdInput) r2AccountIdInput.value = r2Config.account_id || "";
      if (r2BucketInput) r2BucketInput.value = r2Config.bucket || "";
      if (r2AccessKeyInput) r2AccessKeyInput.value = r2Config.access_key || "";
      if (r2SecretKeyInput) r2SecretKeyInput.value = r2Config.secret_key || "";
      if (r2PublicUrlInput) r2PublicUrlInput.value = r2Config.public_url || "";
    }
  } catch (e) {
    console.warn("R2設定の読み込みスキップ:", e);
  }

  // --- Docs Navigation Logic ---
  const docsNavItems = document.querySelectorAll('.docs-nav-item');
  const docsContentArea = document.getElementById('docs-content-area');
  
  if (docsNavItems.length > 0 && docsContentArea) {
    docsNavItems.forEach(item => {
      item.addEventListener('click', () => {
        // Remove active class from all
        docsNavItems.forEach(n => n.classList.remove('active'));
        // Add to clicked
        item.classList.add('active');
        
        // Hide all sections, show target
        const targetId = item.getAttribute('data-target');
        const sections = document.querySelectorAll('.docs-section');
        sections.forEach(sec => sec.classList.remove('active'));
        
        const targetSection = document.getElementById(targetId);
        if (targetSection) {
          targetSection.classList.add('active');
          docsContentArea.scrollTop = 0;
        }
      });
    });
    // Init first section
    const firstTarget = docsNavItems[0].getAttribute('data-target');
    const firstSection = document.getElementById(firstTarget);
    if (firstSection) firstSection.classList.add('active');
  }
});

// ── CF APIキー保存 ──
function saveCfApiKey() {
  const key = cfApiKeyInput.value.trim();
  if (key) {
    localStorage.setItem("cf_api_key", key);
    addLog("✔ CurseForge APIキーを保存しました。", "success");
  } else {
    localStorage.removeItem("cf_api_key");
    addLog("CurseForge APIキーをクリアしました。", "info");
  }
}

function getCfApiKey() {
  return localStorage.getItem("cf_api_key") || "";
}

function clearCfApiKey() {
  cfApiKeyInput.value = "";
  localStorage.removeItem("cf_api_key");
  addLog("CurseForge APIキーをクリアしました。", "info");
}

// ── R2 設定保存 (Rust側に安全に保存) ──
async function saveR2Config() {
  const config = {
    account_id: r2AccountIdInput.value.trim(),
    bucket: r2BucketInput.value.trim(),
    access_key: r2AccessKeyInput.value.trim(),
    secret_key: r2SecretKeyInput.value.trim(),
    public_url: r2PublicUrlInput.value.trim()
  };
  try {
    await invoke("save_r2_config", { config });
    addLog("✔ Cloudflare R2 の設定を保存しました。", "success");
  } catch (e) {
    addLog(`✘ R2設定の保存に失敗: ${e}`, "error");
  }
}

async function clearR2Config() {
  if (r2AccountIdInput) r2AccountIdInput.value = "";
  if (r2BucketInput) r2BucketInput.value = "";
  if (r2AccessKeyInput) r2AccessKeyInput.value = "";
  if (r2SecretKeyInput) r2SecretKeyInput.value = "";
  if (r2PublicUrlInput) r2PublicUrlInput.value = "";
  try {
    await invoke("clear_r2_config");
    addLog("Cloudflare R2 の設定をクリアしました。", "info");
  } catch (e) {
    addLog(`✘ R2設定のクリアに失敗: ${e}`, "error");
  }
}

async function getR2Config() {
  try {
    return await invoke("load_r2_config");
  } catch (e) {
    return null;
  }
}

// ── モード切替 ──
function switchMode(mode) {
  if (currentMode === mode) return;
  
  // モード切替前のアクティブパネルを非表示にする
  const prevPanel = document.querySelector(".mode-panel.active");
  if (prevPanel) {
    prevPanel.classList.remove("active");
    prevPanel.style.display = "none";
  }

  currentMode = mode;

  navModeSync.classList.toggle("active", mode === "sync");
  navModeGenerate.classList.toggle("active", mode === "generate");
  navSettingsBtn.classList.toggle("active", mode === "settings");
  const navAboutBtn = document.getElementById("nav-about-btn");
  if (navAboutBtn) navAboutBtn.classList.toggle("active", mode === "docs");

  // 対象パネルを表示してGSAPでアニメーション
  let targetPanelId;
  if (mode === "sync") targetPanelId = "panel-sync";
  else if (mode === "generate") targetPanelId = "panel-generate";
  else if (mode === "settings") targetPanelId = "panel-settings";
  else if (mode === "docs") targetPanelId = "panel-docs";
  
  const targetPanel = document.getElementById(targetPanelId);
  targetPanel.classList.add("active");
  targetPanel.style.display = "block"; // settings の初期非表示を上書き
  // アニメーションなしで即時表示

  if (mode === "sync") {
    headerTitleText.setAttribute("data-i18n", "header.sync_title");
    headerTitleText.textContent = window.i18n ? window.i18n.t("header.sync_title") : "MODダウンロード同期";
  } else if (mode === "generate") {
    headerTitleText.setAttribute("data-i18n", "header.generate_title");
    headerTitleText.textContent = window.i18n ? window.i18n.t("header.generate_title") : "マニフェスト生成 (リスト抽出)";
  } else if (mode === "settings") {
    headerTitleText.setAttribute("data-i18n", "nav.settings");
    headerTitleText.textContent = window.i18n ? window.i18n.t("nav.settings") : "設定";
  } else if (mode === "docs") {
    headerTitleText.removeAttribute("data-i18n");
    headerTitleText.textContent = "ModLinker の使い方 (説明書)";
  }

  const commonGameDir = document.getElementById("common-game-dir");
  const progressSection = document.getElementById("progress-section");
  if (mode === "docs") {
    if (commonGameDir) commonGameDir.style.display = "none";
    if (progressSection) progressSection.style.display = "none";
  } else {
    if (commonGameDir) commonGameDir.style.display = "block";
    if (progressSection) progressSection.style.display = "block";
  }

  resultsSection.style.display = "none";
  if (diffSection) diffSection.style.display = "none";
  if (manualDlSection) manualDlSection.style.display = "none";
}

// ── テーマ切替 ──
function toggleTheme() {
  const isLight = document.documentElement.classList.toggle("theme-light");
  localStorage.setItem("theme", isLight ? "light" : "dark");
  updateThemeUI(isLight);
}
function updateThemeUI(isLight) {
  if (isLight) {
    themeText.setAttribute("data-i18n", "nav.theme.dark"); if (window.i18n) { themeText.textContent = window.i18n.t("nav.theme.dark"); } else { themeText.textContent = "ダークモード"; }
    themeIcon.innerHTML = `<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"></path>`;
  } else {
    themeText.setAttribute("data-i18n", "nav.theme.light"); if (window.i18n) { themeText.textContent = window.i18n.t("nav.theme.light"); } else { themeText.textContent = "ライトモード"; }
    themeIcon.innerHTML = `
      <circle cx="12" cy="12" r="5"/>
      <line x1="12" y1="1" x2="12" y2="3"/>
      <line x1="12" y1="21" x2="12" y2="23"/>
      <line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/>
      <line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/>
      <line x1="1" y1="12" x2="3" y2="12"/>
      <line x1="21" y1="12" x2="23" y2="12"/>
      <line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/>
      <line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/>
    `;
  }
}

// ── 進捗リスナー ──
async function setupProgressListener() {
  await listen("sync-progress", (event) => {
    const { stage, message, progress } = event.payload;
    updateProgress(progress, message);
    addLog(message, stageToLogLevel(stage));
    updateStatusDot(stage);
  });
}

// ── フォルダ選択 ──
async function handleBrowse() {
  try {
    const selected = await open({
      directory: true, multiple: false,
      title: "ゲームディレクトリ (.minecraft 等) を選択",
    });
    if (selected) gamePathInput.value = selected;
  } catch (err) {
    addLog(`フォルダ選択エラー: ${err}`, "error");
  }
}

async function handleBrowseManifest() {
  try {
    const selected = await open({
      directory: false,
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
      title: "マニフェストファイル (JSON) を選択"
    });
    if (selected) {
      const content = await invoke("read_file_content", { path: selected });
      loadedManifestContent = content;
      manifestInput.value = selected;
      addLog(`マニフェストファイルを読み込みました: ${selected}`, "info");
    }
  } catch (err) {
    addLog(`ファイル読み込みエラー: ${err}`, "error");
  }
}

// ── マニフェスト生成 (スキャン) ──
async function handleGenerateManifest() {
  const gamePath = gamePathInput.value.trim();
  if (!gamePath) {
    addLog("⚠ ゲームディレクトリを入力してください。", "warning");
    gamePathInput.focus();
    return;
  }

  if (isProcessing) return;
  setProcessingState(startGenerateBtn, true, "スキャン中...");
  resultsSection.style.display = "none";
  addLog(`マニフェスト生成開始: ${gamePath}`, "info");

  const cfKey = getCfApiKey();
  if (cfKey) addLog("CurseForge APIキーあり → CF照合を実行します。", "info");
  else addLog("CurseForge APIキーなし → Modrinth のみで照合します。", "info");

  try {
    const manifest = await invoke("sync_mods", {
      gamePath: gamePath,
      cfApiKey: cfKey || null
    });

    // R2設定があれば、API未登録(unknown)なMODをR2へ自動アップロード
    const r2Config = await getR2Config();
    if (r2Config && r2Config.account_id && r2Config.bucket && r2Config.access_key && r2Config.secret_key) {
      for (let i = 0; i < manifest.length; i++) {
        const mod = manifest[i];
        if (mod.source === "unknown" && !mod.download_url) {
          const separator = gamePath.includes("\\") ? "\\" : "/";
          let dir = mod.target_dir.replace(/^[\/\\]+/, "");
          const filePath = `${gamePath}${separator}${dir}${separator}${mod.file_name}`;
          
          addLog(`API未登録MODをR2へ自動アップロードします: ${mod.file_name}`, "info");
          try {
            const publicUrl = await invoke("upload_to_r2", {
              filePath: filePath
            });
            manifest[i].download_url = publicUrl;
            manifest[i].source = "manual"; // R2からのダウンロード(manual)扱いに変更
            addLog(`✔ R2アップロード完了: ${publicUrl}`, "success");
          } catch (r2Err) {
            addLog(`✘ R2アップロード失敗 (${mod.file_name}): ${r2Err}`, "error");
          }
        }
      }
    }

    lastManifest = manifest;
    renderResults(manifest);
    loadedManifestContent = JSON.stringify(manifest, null, 2);
    manifestInput.value = "(自動生成されたマニフェスト)";

    // ブラウザを開くプロンプトはダウンロード時のみ表示するため、ここでは何もしない
  } catch (err) {
    addLog(`✘ エラー: ${err}`, "error");
    updateProgress(0, "エラーが発生しました");
    updateStatusDot("error");
  } finally {
    setProcessingState(startGenerateBtn, false, "スキャン開始");
  }
}

// ── 差分チェック ──
async function handleCheckDiff() {
  const gamePath = gamePathInput.value.trim();
  if (!gamePath) {
    addLog("⚠ ゲームディレクトリを入力してください。", "warning");
    gamePathInput.focus();
    return;
  }

  const manifestText = loadedManifestContent.trim();
  if (!manifestText) {
    addLog("⚠ マニフェスト(JSON)が選択されていません。", "warning");
    return;
  }

  const skipR2 = skipR2Checkbox.checked;

  if (isProcessing) return;
  setProcessingState(startDiffBtn, true, "チェック中...");
  diffSection.style.display = "none";
  manualDlSection.style.display = "none";
  addLog(`差分チェック開始: ${gamePath}`, "info");

  try {
    const diffResults = await invoke("check_sync_diff", {
      gamePath: gamePath,
      manifestJson: manifestText,
      skipR2: skipR2
    });
    renderDiffResults(diffResults);
  } catch (err) {
    addLog(`✘ 差分チェックエラー: ${err}`, "error");
    updateProgress(0, "エラーが発生しました");
    updateStatusDot("error");
  } finally {
    setProcessingState(startDiffBtn, false, "差分をチェック");
  }
}

function renderDiffResults(diffResults) {
  diffSection.style.display = "block";
  diffTbody.innerHTML = "";
  currentDiffTargets = [];
  currentManualTargets = [];

  let dlCount = 0, skipCount = 0, upToDateCount = 0, manualCount = 0;

  for (const entry of diffResults) {
    const tr = document.createElement("tr");
    let statusBadge = "";
    let sourceBadge = makeSourceBadge(entry.source);

    if (entry.action === "download") {
      statusBadge = `<span class="status-badge unmatched">↓ DL対象</span>`;
      dlCount++;
      currentDiffTargets.push({
        file_name: entry.file_name,
        target_dir: entry.target_dir,
        url: entry.url
      });
    } else if (entry.action === "manual") {
      statusBadge = `<span class="status-badge" style="background: rgba(255, 165, 0, 0.12); color: orange;">🔗 手動DL</span>`;
      manualCount++;
      currentManualTargets.push(entry);
    } else if (entry.action === "skip") {
      statusBadge = `<span class="status-badge" style="background: rgba(255, 165, 0, 0.12); color: orange;">⚠ スキップ</span>`;
      skipCount++;
    } else if (entry.action === "up_to_date") {
      statusBadge = `<span class="status-badge matched">✔ 最新</span>`;
      upToDateCount++;
    }

    const urlCell = entry.url
      ? `<a class="url-link" href="${escapeHtml(entry.url)}" target="_blank">Link</a>`
      : entry.page_url
        ? `<a class="url-link" href="${escapeHtml(entry.page_url)}" target="_blank">手動DL</a>`
        : `<span style="color: var(--text-muted);">—</span>`;

    tr.innerHTML = `
      <td><span class="dir-badge">${escapeHtml(entry.target_dir)}/</span></td>
      <td>${statusBadge}</td>
      <td class="file-name-cell">${escapeHtml(entry.file_name)}</td>
      <td>${sourceBadge}</td>
      <td>${urlCell}</td>
    `;
    diffTbody.appendChild(tr);
  }

  diffStats.innerHTML = `
    <div class="stat-item">
      <span class="stat-label">DL対象:</span>
      <span class="stat-value" style="color: var(--accent-error);">${dlCount}</span>
    </div>
    <div class="stat-item">
      <span class="stat-label">最新:</span>
      <span class="stat-value" style="color: var(--accent-success);">${upToDateCount}</span>
    </div>
    <div class="stat-item">
      <span class="stat-label">手動DL:</span>
      <span class="stat-value" style="color: orange;">${manualCount}</span>
    </div>
    <div class="stat-item">
      <span class="stat-label">スキップ:</span>
      <span class="stat-value" style="color: var(--text-muted);">${skipCount}</span>
    </div>
  `;

  executeDownloadBtn.disabled = (dlCount === 0 && manualCount === 0);
  if (dlCount > 0) {
    executeDownloadBtn.textContent = `ダウンロード開始 (${dlCount} 個)`;
  } else if (manualCount > 0) {
    executeDownloadBtn.textContent = `手動DLページを開く (${manualCount} 個)`;
  } else {
    executeDownloadBtn.textContent = "ダウンロード不要";
  }

  // gsap アニメーション削除

  // 手動DLリンク一覧
  if (currentManualTargets.length > 0) {
    manualDlSection.style.display = "block";
    manualDlList.innerHTML = currentManualTargets.map(item => `
      <div style="display: flex; align-items: center; gap: 10px; padding: 8px 0; border-bottom: 1px solid var(--glass-border);">
        <span class="dir-badge">${escapeHtml(item.target_dir)}/</span>
        <span class="file-name-cell" style="flex: 1;">${escapeHtml(item.file_name)}</span>
        ${item.page_url ? `<a class="url-link" href="${escapeHtml(item.page_url)}" target="_blank" style="white-space: nowrap;">🔗 ページを開く</a>` : `<span style="color: var(--text-muted); font-size: 12px;">リンクなし</span>`}
      </div>
    `).join("");
    // gsap アニメーション削除
  } else {
    manualDlSection.style.display = "none";
  }
}

// ── 実際のダウンロード ──
async function handleExecuteDownload() {
  const gamePath = gamePathInput.value.trim();
  if (currentDiffTargets.length === 0 && currentManualTargets.length === 0) return;

  if (isProcessing) return;
  setProcessingState(executeDownloadBtn, true, "処理中...");

  let hasError = false;

  if (currentDiffTargets.length > 0) {
    addLog(`ダウンロードを開始します（${currentDiffTargets.length} 個）`, "info");
    try {
      await invoke("execute_download", {
        gamePath: gamePath,
        targetsJson: JSON.stringify(currentDiffTargets)
      });
    } catch (err) {
      addLog(`✘ ダウンロードエラー: ${err}`, "error");
      updateProgress(0, "エラーが発生しました");
      updateStatusDot("error");
      hasError = true;
    }
  }

  if (!hasError && currentManualTargets.length > 0) {
    setProcessingState(executeDownloadBtn, false, "完了");
    // v2.0.0: 📥 スマート手動MOD自動回収モーダルを起動
    openWatchModal();
    return;
  }

  setProcessingState(executeDownloadBtn, false, "完了");
}

// ── ソースバッジ生成 ──
function makeSourceBadge(source) {
  switch (source) {
    case "modrinth":
      return `<span class="status-badge" style="background: rgba(30, 200, 120, 0.12); color: #1ec878;">Modrinth</span>`;
    case "curseforge":
      return `<span class="status-badge" style="background: rgba(240, 100, 40, 0.12); color: #f06428;">CurseForge</span>`;
    case "manual":
      return `<span class="status-badge" style="background: rgba(255, 165, 0, 0.12); color: orange;">手動DL</span>`;
    case "unknown":
      return `<span class="status-badge" style="background: rgba(128, 128, 128, 0.12); color: var(--text-muted);">アシスト検索</span>`;
    default:
      return `<span class="status-badge" style="background: rgba(128, 128, 128, 0.12); color: var(--text-muted);">不明</span>`;
  }
}

// ── UI状態管理 ──
function setProcessingState(btn, processing, text) {
  isProcessing = processing;
  btn.disabled = processing;
  if (processing) {
    btn.classList.add("running");
    updateProgress(0, "準備中...");
    updateStatusDot("running");
  } else {
    btn.classList.remove("running");
  }
  const span = btn.querySelector("span");
  if (span) span.textContent = text;
  else btn.textContent = text;
}

function updateProgress(ratio, label) {
  const pct = Math.round(ratio * 100);
  progressBar.style.width = `${pct}%`;
  progressPercent.textContent = `${pct}%`;
  if (label) progressLabel.textContent = label;
}

function updateStatusDot(stage) {
  const dot = statusIndicator.querySelector(".status-dot");
  const text = statusIndicator.querySelector("span");
  dot.className = "status-dot";
  switch (stage) {
    case "scanning": case "hashing": case "querying": case "building": case "running":
      dot.classList.add("running"); text.textContent = "処理中"; break;
    case "done":
      dot.classList.add("done"); text.textContent = "完了"; break;
    case "error":
      dot.classList.add("error"); text.textContent = "エラー"; break;
    default:
      dot.classList.add("idle"); text.textContent = "待機中";
  }
}

// ── ログ関連 ──
function getActiveLogConsole() {
  if (currentMode === "generate") return logConsoleGenerate;
  return logConsoleSync; // sync, settings どちらもsync側に出す
}

function addLog(message, level = "info") {
  const console = getActiveLogConsole();
  if (!console) return;
  const entry = document.createElement("div");
  entry.className = `log-entry ${level}`;
  const now = new Date();
  const timeStr = [now.getHours(), now.getMinutes(), now.getSeconds()]
    .map(n => n.toString().padStart(2, "0")).join(":");
  entry.innerHTML = `<span class="log-time">${timeStr}</span><span class="log-msg">${escapeHtml(message)}</span>`;
  console.appendChild(entry);
  console.scrollTop = console.scrollHeight;
}

function clearLog(target) {
  const console = target === "generate" ? logConsoleGenerate : logConsoleSync;
  if (!console) return;
  console.innerHTML = "";
  const entry = document.createElement("div");
  entry.className = "log-entry info";
  const msg = window.i18n ? window.i18n.t("log.cleared") : "ログをクリアしました。";
  entry.innerHTML = `<span class="log-time">--:--:--</span><span class="log-msg">${msg}</span>`;
  console.appendChild(entry);
}

function stageToLogLevel(stage) {
  switch (stage) {
    case "done": return "success";
    case "warning": return "warning";
    case "error": return "error";
    default: return "progress";
  }
}

// ── 結果テーブル描画 ──
function renderResults(manifest) {
  if (!manifest || manifest.length === 0) {
    resultsSection.style.display = "none";
    modCountBadge.textContent = "0 Files";
    return;
  }

  resultsSection.style.display = "block";
  resultsTbody.innerHTML = "";

  const modrinthCount = manifest.filter(m => m.source === "modrinth").length;
  const cfCount = manifest.filter(m => m.source === "curseforge").length;
  const manualCount = manifest.filter(m => m.source === "manual").length;
  const unknownCount = manifest.filter(m => m.source === "unknown").length;
  modCountBadge.textContent = `${manifest.length} Files`;

  resultsStats.innerHTML = `
    <div class="stat-item">
      <span class="stat-dot" style="background: #1ec878;"></span>
      <span class="stat-label">Modrinth:</span>
      <span class="stat-value">${modrinthCount}</span>
    </div>
    <div class="stat-item">
      <span class="stat-dot" style="background: #f06428;"></span>
      <span class="stat-label">CurseForge:</span>
      <span class="stat-value">${cfCount}</span>
    </div>
    <div class="stat-item">
      <span class="stat-dot" style="background: orange;"></span>
      <span class="stat-label">手動DL:</span>
      <span class="stat-value">${manualCount}</span>
    </div>
    <div class="stat-item">
      <span class="stat-dot unmatched"></span>
      <span class="stat-label">アシスト検索:</span>
      <span class="stat-value">${unknownCount}</span>
    </div>
  `;

  for (const mod of manifest) {
    const tr = document.createElement("tr");
    const sourceBadge = makeSourceBadge(mod.source);

    const urlCell = mod.download_url
      ? `<a class="url-link" href="${escapeHtml(mod.download_url)}" target="_blank">CDN Link</a>`
      : mod.page_url
        ? `<a class="url-link" href="${escapeHtml(mod.page_url)}" target="_blank">手動DL</a>`
        : `<span style="color: var(--text-muted);">—</span>`;

    tr.innerHTML = `
      <td><span class="dir-badge">${escapeHtml(mod.target_dir)}/</span></td>
      <td>${sourceBadge}</td>
      <td class="file-name-cell">${escapeHtml(mod.file_name)}</td>
      <td>${mod.project_name ? escapeHtml(mod.project_name) : '<span style="color: var(--text-muted);">—</span>'}</td>
      <td>${urlCell}</td>
    `;
    resultsTbody.appendChild(tr);
  }

  // ── R2 アップロード提案 ──
  const r2Prompt = document.getElementById("r2-upload-prompt");
  const r2Desc = document.getElementById("r2-upload-desc");
  const r2Actions = document.getElementById("r2-upload-actions");
  
  if (unknownCount > 0 && r2Prompt) {
    r2Prompt.style.display = "block";
    r2Desc.textContent = window.i18n ? window.i18n.t("r2_upload.desc", [unknownCount]) : `${unknownCount} 個のMODが不明です...`;
    
    r2Actions.innerHTML = "";
    const uploadBtn = document.createElement("button");
    uploadBtn.className = "btn-primary";
    uploadBtn.textContent = window.i18n ? window.i18n.t("r2_upload.btn", [unknownCount]) : `R2にアップロード (${unknownCount})`;
    uploadBtn.addEventListener("click", handleR2UploadUnknowns);
    r2Actions.appendChild(uploadBtn);
  } else if (r2Prompt) {
    r2Prompt.style.display = "none";
  }
}

async function handleR2UploadUnknowns() {
  const gamePath = gamePathInput.value.trim();
  if (!gamePath) return;

  const r2Config = await invoke("load_r2_config");
  if (!r2Config) {
    alert(window.i18n ? window.i18n.t("r2_upload.no_config_desc") : "R2の設定が必要です");
    switchMode("settings");
    return;
  }

  const unknowns = lastManifest.filter(m => m.source === "unknown");
  if (unknowns.length === 0) return;

  const btn = document.querySelector("#r2-upload-actions button");
  if (btn) btn.disabled = true;

  try {
    let successCount = 0;
    for (let i = 0; i < unknowns.length; i++) {
      const mod = unknowns[i];
      if (btn) btn.textContent = window.i18n ? window.i18n.t("r2_upload.uploading", [i + 1, unknowns.length]) : `アップロード中... (${i+1}/${unknowns.length})`;
      
      const filePath = `${gamePath}\\${mod.target_dir}\\${mod.file_name}`;
      try {
        const publicUrl = await invoke("upload_to_r2", { filePath });
        mod.download_url = publicUrl;
        mod.source = "unknown"; // r2にしてもいいが、一応unknownのままURLを付与
        successCount++;
      } catch (e) {
        addLog(`✘ ${mod.file_name}: ${e}`, "error");
      }
    }
    
    // マニフェスト再保存
    const autoSavePath = `${gamePath}\\modlinker-manifest.json`;
    await invoke("save_manifest_to_file", { path: autoSavePath, content: JSON.stringify(lastManifest, null, 2) });
    
    addLog(window.i18n ? window.i18n.t("r2_upload.done", [successCount]) : `✔ ${successCount} 個のMODをR2にアップロードしました。`, "success");
    
    // 再描画
    renderResults(lastManifest);
  } catch (err) {
    addLog(window.i18n ? window.i18n.t("r2_upload.error", [err]) : `✘ エラー: ${err}`, "error");
  } finally {
    if (btn) btn.disabled = false;
  }
}

// ── マニフェスト保存・エクスポート ──
async function handleSaveManifest() {
  if (!lastManifest || lastManifest.length === 0) {
    addLog("⚠ 保存するマニフェストがありません。", "warning");
    return;
  }
  try {
    const filePath = await window.__TAURI__.dialog.save({
      filters: [{ name: 'JSON', extensions: ['json'] }],
      defaultPath: "modlinker-manifest.json",
      title: "マニフェストを保存"
    });
    if (filePath) {
      const json = JSON.stringify(lastManifest, null, 2);
      await invoke("save_manifest_to_file", { path: filePath, content: json });
      addLog(`✔ マニフェストを保存しました: ${filePath}`, "success");
    }
  } catch (err) {
    addLog(`✘ 保存エラー: ${err}`, "error");
  }
}

function handleExportManifest() {
  if (!lastManifest || lastManifest.length === 0) {
    addLog("⚠ エクスポートするマニフェストがありません。", "warning");
    return;
  }
  const json = JSON.stringify(lastManifest, null, 2);
  navigator.clipboard.writeText(json)
    .then(() => addLog("✔ マニフェストJSONをクリップボードにコピーしました。", "success"))
    .catch(() => addLog("⚠ クリップボードへのコピーに失敗しました。", "warning"));
}

function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}



window.addEventListener("DOMContentLoaded", () => {
  // --- Copy Button Logic ---
  document.addEventListener("click", async (e) => {
    const btn = e.target.closest(".copy-btn");
    if (btn) {
      const textToCopy = btn.getAttribute("data-copy");
      if (textToCopy) {
        try {
          await navigator.clipboard.writeText(textToCopy);
          const originalText = btn.textContent;
          btn.textContent = window.i18n ? window.i18n.t("common.copied") : "コピーしました！";
          setTimeout(() => { btn.textContent = originalText; }, 2000);
        } catch (err) {
          console.error("Failed to copy text: ", err);
        }
      }
    }
  });

});

// ═══════════════════════════════════════════════════════════
//  ModLinker v2.0.0 – 公式ランチャー連携 & スマート自動回収
// ═══════════════════════════════════════════════════════════

// ── v2 DOM要素 ──
let navLauncherBtn;
let launcherManifestInput, launcherBrowseBtn, profileNameInput;
let launcherDetectBox, detectLoaderValue, detectMcValue, detectJavaValue, detectModsValue;
let zgcToggleCheckbox, jvmPlanPreview, jvmPlanArgs;
let startLauncherSyncBtn, launcherSummarySection, launcherSummaryBody;
let selectedMemoryMb = 8192;

// 待機モーダル
let watchModal, watchPhaseBadge, watchTargetList, watchDirPath, watchChangeDirBtn;
let watchScanNowBtn, watchPickFileBtn, watchDropZone, watchStatusMsg, watchCancelBtn, watchDoneBtn, watchIcon;

// 待機モーダル状態
let watchSession = {
  sessionId: null,
  targets: [],        // [{file_name, target_dir, sha512}]
  gamePath: null,
  watchDir: null,
  isClosed: false,
};

// ランチャー状態
let launcherManifestContent = "";
let launcherDetected = null; // detect_manifest_info の結果

// ── ナビ統合: switchMode に launcher を追加 ──
const _origSwitchMode = switchMode;
switchMode = function (mode) {
  if (mode === "launcher") {
    if (currentMode === "launcher") return;
    const prevPanel = document.querySelector(".mode-panel.active");
    if (prevPanel) {
      prevPanel.classList.remove("active");
      prevPanel.style.display = "none";
    }
    currentMode = mode;
    navModeSync.classList.remove("active");
    navModeGenerate.classList.remove("active");
    navSettingsBtn.classList.remove("active");
    const aboutBtn = document.getElementById("nav-about-btn");
    if (aboutBtn) aboutBtn.classList.remove("active");
    if (navLauncherBtn) navLauncherBtn.classList.add("active");

    const targetPanel = document.getElementById("panel-launcher");
    targetPanel.classList.add("active");
    targetPanel.style.display = "block";

    headerTitleText.removeAttribute("data-i18n");
    headerTitleText.textContent = window.i18n ? window.i18n.t("header.launcher_title") : "ワンクリック同期 (公式ランチャー連携)";

    const commonGameDir = document.getElementById("common-game-dir");
    const progressSection = document.getElementById("progress-section");
    if (commonGameDir) commonGameDir.style.display = "block";
    if (progressSection) progressSection.style.display = "block";

    resultsSection.style.display = "none";
    if (diffSection) diffSection.style.display = "none";
    if (manualDlSection) manualDlSection.style.display = "none";
    return;
  }
  // 既存のモード切替 (ランチャーnavを必ず消す)
  if (navLauncherBtn) navLauncherBtn.classList.remove("active");
  _origSwitchMode(mode);
};

// ═══════════════════════════════════════════════════════════
//  ランチャーワンクリック同期
// ═══════════════════════════════════════════════════════════

window.addEventListener("DOMContentLoaded", async () => {
  navLauncherBtn = document.getElementById("nav-launcher-btn");
  launcherManifestInput = document.getElementById("launcher-manifest-input");
  launcherBrowseBtn = document.getElementById("launcher-browse-btn");
  profileNameInput = document.getElementById("profile-name-input");
  launcherDetectBox = document.getElementById("launcher-detect-box");
  detectLoaderValue = document.getElementById("detect-loader-value");
  detectMcValue = document.getElementById("detect-mc-value");
  detectJavaValue = document.getElementById("detect-java-value");
  detectModsValue = document.getElementById("detect-mods-value");
  zgcToggleCheckbox = document.getElementById("zgc-toggle-checkbox");
  jvmPlanPreview = document.getElementById("jvm-plan-preview");
  jvmPlanArgs = document.getElementById("jvm-plan-args");
  startLauncherSyncBtn = document.getElementById("start-launcher-sync-btn");
  launcherSummarySection = document.getElementById("launcher-summary-section");
  launcherSummaryBody = document.getElementById("launcher-summary-body");

  watchModal = document.getElementById("watch-modal");
  watchPhaseBadge = document.getElementById("watch-phase-badge");
  watchTargetList = document.getElementById("watch-target-list");
  watchDirPath = document.getElementById("watch-dir-path");
  watchChangeDirBtn = document.getElementById("watch-change-dir-btn");
  watchScanNowBtn = document.getElementById("watch-scan-now-btn");
  watchPickFileBtn = document.getElementById("watch-pick-file-btn");
  watchDropZone = document.getElementById("watch-drop-zone");
  watchStatusMsg = document.getElementById("watch-status-msg");
  watchCancelBtn = document.getElementById("watch-cancel-btn");
  watchDoneBtn = document.getElementById("watch-done-btn");
  watchIcon = document.getElementById("watch-icon");

  if (navLauncherBtn) navLauncherBtn.addEventListener("click", () => switchMode("launcher"));
  if (launcherBrowseBtn) launcherBrowseBtn.addEventListener("click", handleLauncherBrowseManifest);
  if (launcherManifestInput) launcherManifestInput.addEventListener("click", handleLauncherBrowseManifest);
  if (startLauncherSyncBtn) startLauncherSyncBtn.addEventListener("click", handleLauncherSync);
  if (zgcToggleCheckbox) zgcToggleCheckbox.addEventListener("change", refreshJvmPreview);
  if (watchCancelBtn) watchCancelBtn.addEventListener("click", cancelWatch);
  if (watchDoneBtn) watchDoneBtn.addEventListener("click", closeWatchModal);
  if (watchChangeDirBtn) watchChangeDirBtn.addEventListener("click", changeWatchDir);
  if (watchScanNowBtn) watchScanNowBtn.addEventListener("click", scanNowHandler);
  if (watchPickFileBtn) watchPickFileBtn.addEventListener("click", pickFileHandler);
  if (watchDropZone) {
    watchDropZone.addEventListener("click", pickFileHandler);
    watchDropZone.addEventListener("dragover", (e) => {
      e.preventDefault();
      watchDropZone.classList.add("drag-over");
    });
    watchDropZone.addEventListener("dragleave", () => watchDropZone.classList.remove("drag-over"));
    watchDropZone.addEventListener("drop", (e) => e.preventDefault()); // Tauri drag-drop イベントに委譲
  }

  // メモリチップ
  document.querySelectorAll(".memory-chip").forEach(chip => {
    chip.addEventListener("click", () => {
      document.querySelectorAll(".memory-chip").forEach(c => c.classList.remove("active"));
      chip.classList.add("active");
      selectedMemoryMb = parseInt(chip.dataset.mb, 10);
      refreshJvmPreview();
    });
  });
  // 既定は 8GB をアクティブに
  const defChip = document.querySelector('.memory-chip[data-mb="8192"]');
  if (defChip) defChip.classList.add("active");

  // 進捗リスナー (ランチャー連携)
  if (listen) {
    await listen("launcher-sync-progress", (event) => {
      const { phase, message, progress } = event.payload;
      updateProgress(progress, message);
      addLog(message, phase === "done" ? "success" : phase === "error" ? "error" : "progress");
      updateStatusDot(phase === "done" ? "done" : phase === "error" ? "error" : "running");
    });
    await listen("watcher-progress", (event) => {
      handleWatcherProgress(event.payload);
    });
  }

  // Tauri 標準 drag-drop イベント連携 (複数一括対応)
  if (listen) {
    try {
      await listen("tauri://drag-drop", (event) => {
        const paths = event.payload?.paths || [];
        if (watchModal && watchModal.style.display !== "none" && paths.length > 0) {
          placeFromPaths(paths);
        }
      });
    } catch (e) {
      console.warn("drag-drop listen 失敗:", e);
    }
  }
});

// マニフェスト選択 (ランチャーパネル)
async function handleLauncherBrowseManifest() {
  try {
    const selected = await open({
      directory: false, multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
      title: "マニフェストファイル (JSON) を選択"
    });
    if (selected) {
      const content = await invoke("read_file_content", { path: selected });
      launcherManifestContent = content;
      launcherManifestInput.value = selected;
      addLog(`マニフェストを読み込みました: ${selected}`, "info");
      await detectManifest();
    }
  } catch (err) {
    addLog(`ファイル読み込みエラー: ${err}`, "error");
  }
}

// マニフェスト検出 (ローダー/バージョン/MOD数)
async function detectManifest() {
  if (!launcherManifestContent.trim()) return;
  try {
    const info = await invoke("detect_manifest_info", { manifestJson: launcherManifestContent });
    launcherDetected = info;

    launcherDetectBox.style.display = "flex";
    const loader = info.loader;
    detectLoaderValue.textContent = loader
      ? ({ fabric: "Fabric", forge: "Forge", neoforge: "NeoForge", vanilla: "Vanilla" }[loader.kind] || loader.kind)
      : (window.i18n?.t("launcher.no_loader") || "指定なし (MOD同期のみ)");
    detectMcValue.textContent = loader ? loader.mc_version : "—";
    const recJava = info.tuning?.recommended_java ?? (loader ? 21 : 8);
    detectJavaValue.textContent = `Java ${recJava}`;

    // ローダーが指定されていればバージョンも解決して表示
    if (loader && loader.mc_version) {
      try {
        const ver = await invoke("resolve_loader_version", {
          loader: { kind: loader.kind, mc_version: loader.mc_version, loader_version: loader.loader_version ?? null }
        });
        detectLoaderValue.textContent += ` (最新: ${ver})`;
      } catch (e) { /* 表示のみなので失敗許容 */ }
    }
    detectModsValue.textContent = `${info.mod_count}`;

    // v1形式 (ローダー情報なし) → 手動指定UIを表示
    const overrideGroup = document.getElementById("loader-override-group");
    if (overrideGroup) overrideGroup.style.display = loader ? "none" : "flex";

    // Java 実行環境チェック
    try {
      const rt = await invoke("resolve_java", { requiredMajor: recJava });
      if (rt) {
        detectJavaValue.textContent = `Java ${rt.major_version} (v${rt.version_string}) ✓ 内蔵/システム`;
      } else {
        detectJavaValue.textContent = `Java ${recJava} — ${window.i18n?.t("launcher.java_not_found") || "未検出 (公式ランチャー内蔵Javaを探索)"}`;
        detectJavaValue.style.color = "var(--accent-warning)";
      }
    } catch (e) { /* noop */ }

    refreshJvmPreview();
    addLog(`ローダー検出: ${loader ? loader.kind : "なし"} / MC ${loader ? loader.mc_version : "-"} / MOD ${info.mod_count} 件`, "info");
  } catch (err) {
    addLog(`マニフェスト解析エラー: ${err}`, "error");
  }
}

// JVM プランプレビュー更新
function currentTuning() {
  const recJava = launcherDetected?.tuning?.recommended_java ?? (launcherDetected?.loader ? 21 : 8);
  return {
    recommended_java: recJava,
    gc: launcherDetected?.tuning?.gc || "auto",
    memory_mb: selectedMemoryMb,
    always_pre_touch: launcherDetected?.tuning?.always_pre_touch ?? false,
  };
}

async function refreshJvmPreview() {
  if (!jvmPlanPreview) return;
  if (!launcherManifestContent.trim()) { jvmPlanPreview.style.display = "none"; return; }
  try {
    const plan = await invoke("plan_jvm_args", {
      tuningJson: currentTuning(),
      zgcEnabled: zgcToggleCheckbox?.checked ?? false,
      javaMajor: null,
    });
    jvmPlanArgs.textContent = plan.args.join(" ");
    jvmPlanPreview.style.display = "block";
    if (plan.notes && plan.notes.length > 0) {
      plan.notes.forEach(n => addLog(`ℹ JVM: ${n}`, "info"));
    }
  } catch (e) {
    jvmPlanPreview.style.display = "none";
  }
}

// ワンクリック同期実行
async function handleLauncherSync() {
  const gamePath = gamePathInput.value.trim();
  if (!gamePath) {
    addLog("⚠ ゲームディレクトリ (.minecraft 等) を入力してください。", "warning");
    gamePathInput.focus();
    return;
  }
  if (!launcherManifestContent.trim()) {
    addLog("⚠ マニフェストを選択してください。", "warning");
    return;
  }
  const profileName = profileNameInput.value.trim() || "ModLinker Profile";

  // 事前プロセスチェック (UI で即時警告)
  try {
    const proc = await invoke("check_launcher_running_cmd");
    if (proc.running) {
      alert(window.i18n?.t("launcher.launcher_running_alert") ||
        `⛔ Minecraft ランチャーが起動中です (${proc.matched_name})。\n設定の破損を防ぐため、ランチャーを終了してから実行してください。`);
      return;
    }
  } catch (e) { /* チェック失敗は続行 (Rust側でも防御) */ }

  if (isProcessing) return;
  setProcessingState(startLauncherSyncBtn, true, "同期中...");
  launcherSummarySection.style.display = "none";
  addLog(`⚡ ワンクリック同期開始: プロファイル「${profileName}」`, "info");

  // ローダー手動指定 (マニフェストに無い場合)
  let loaderOverride = launcherDetected?.loader ? {
    kind: launcherDetected.loader.kind,
    mc_version: launcherDetected.loader.mc_version,
    loader_version: launcherDetected.loader.loader_version ?? null,
  } : null;
  const kindSel = document.getElementById("loader-kind-select");
  const mcInput = document.getElementById("loader-mc-version-input");
  if (!loaderOverride && kindSel && kindSel.value !== "none") {
    const mc = mcInput?.value.trim() || "";
    if (!mc) {
      addLog("⚠ MCバージョンを入力してください (例: 1.21.1)。", "warning");
      setProcessingState(startLauncherSyncBtn, false, "⚡ ワンクリック同期を開始");
      const span = startLauncherSyncBtn.querySelector("span");
      if (span) span.textContent = window.i18n?.t("launcher.start") || "⚡ ワンクリック同期を開始";
      return;
    }
    loaderOverride = { kind: kindSel.value, mc_version: mc, loader_version: null };
  }

  try {
    const summary = await invoke("run_launcher_sync", {
      gamePath: gamePath,
      manifestJson: launcherManifestContent,
      options: {
        profileName: profileName,
        zgcEnabled: zgcToggleCheckbox?.checked ?? false,
        loaderOverride: loaderOverride,
        skipUnofficial: false,
      }
    });
    renderLauncherSummary(summary);
    addLog(`✅ 完了! 公式ランチャーで「${profileName}」を選んで起動できます。`, "success");
  } catch (err) {
    addLog(`✘ 同期エラー: ${err}`, "error");
    updateProgress(0, "エラーが発生しました");
    updateStatusDot("error");
    // ZGC起因の可能性がある場合はフォールバックプランを提示
    try {
      const fallback = await invoke("plan_jvm_fallback", { tuningJson: currentTuning(), javaMajor: null });
      addLog(`🛟 フォールバックプラン (G1GC): ${fallback.args.join(" ")}`, "warning");
    } catch (e) { /* noop */ }
  } finally {
    setProcessingState(startLauncherSyncBtn, false, "⚡ ワンクリック同期を開始");
    if (startLauncherSyncBtn) {
      const span = startLauncherSyncBtn.querySelector("span");
      if (span) span.textContent = window.i18n?.t("launcher.start") || "⚡ ワンクリック同期を開始";
    }
  }
}

function renderLauncherSummary(s) {
  launcherSummarySection.style.display = "block";
  const T = (k, fb) => (window.i18n ? window.i18n.t(k) : fb);
  const rows = [
    [T("summary.profile", "プロファイル"), s.profile_key, true],
    [T("summary.game_dir", "プロファイル用 gameDir"), s.profile_game_dir],
    [T("summary.loader", "ローダー"), s.loader_version_id || "—"],
    [T("summary.java", "Java"), s.java_executable || "システム既定"],
    [T("summary.jvm_args", "JVM引数 (型安全生成)"), s.jvm_plan.args.join(" ")],
    [T("summary.gc_memory", "GC / メモリ"), `${s.jvm_plan.gc_applied} / ${s.jvm_plan.memory_mb}MB`],
    [T("summary.downloaded", "DL済み MOD"), `${s.downloaded}`],
    [T("summary.skipped", "スキップ"), `${s.skipped}`],
    [T("summary.failed", "失敗"), s.failed.length > 0 ? s.failed.join(", ") : "—"],
    [T("summary.backup", "profiles バックアップ"), s.backup_path || "—"],
  ];
  launcherSummaryBody.innerHTML = rows.map(([k, v, ok]) =>
    `<div class="summary-row"><span class="summary-key">${escapeHtml(String(k))}</span><span class="summary-val ${ok ? "summary-ok" : ""}">${escapeHtml(String(v ?? ""))}</span></div>`
  ).join("");
  if (s.jvm_plan.notes?.length) {
    launcherSummaryBody.innerHTML += `<div class="summary-row"><span class="summary-key">${escapeHtml(T("summary.notes", "メモ"))}</span><span class="summary-val">${escapeHtml(s.jvm_plan.notes.join(" / "))}</span></div>`;
  }
}

// ═══════════════════════════════════════════════════════════
//  📥 スマート手動MOD自動回収 (待機モーダル)
// ═══════════════════════════════════════════════════════════

async function openWatchModal() {
  const gamePath = gamePathInput.value.trim();
  if (!gamePath || currentManualTargets.length === 0) return;

  // 手動DL対象のターゲットを構築 (ハッシュ一致のみで判定)
  watchSession.targets = currentManualTargets
    .filter(m => m.sha512)
    .map(m => ({ file_name: m.file_name, target_dir: m.target_dir, sha512: m.sha512, page_url: m.page_url }));

  if (watchSession.targets.length === 0) {
    // ハッシュなしの古いマニフェスト → 従来動作 (ブラウザを開くだけ)
    addLog("⚠ マニフェストにSHA-512が含まれないため自動回収は使えません (v1形式)。ブラウザで手動DLしてください。", "warning");
    for (const item of currentManualTargets) {
      if (item.page_url) await invoke("open_browser", { url: item.page_url }).catch(() => {});
    }
    return;
  }

  watchSession.gamePath = gamePath;
  watchSession.isClosed = false;

  // 監視先を自動特定 (設定 > ブラウザPreferences > OS標準)
  let dir;
  try {
    const resolved = await invoke("resolve_download_dir_cmd", { preferred: localStorage.getItem("watch_dir") || null });
    dir = resolved.path;
    watchSession.autoSource = resolved.source;
    if (resolved.browser) addLog(`📁 ${resolved.browser} のダウンロード先を検出: ${dir}`, "info");
  } catch (e) {
    dir = null;
  }

  // 監視開始
  try {
    const res = await invoke("start_download_watcher", {
      targetsJson: JSON.stringify(watchSession.targets),
      watchDir: dir,
      destRoot: gamePath,
    });
    watchSession.sessionId = res.session_id;
    watchSession.watchDir = res.watch_dir;
    watchDirPath.textContent = res.watch_dir;
    addLog(`📥 ダウンロード待機モーダルを表示 (監視先: ${res.watch_dir})`, "info");
  } catch (err) {
    addLog(`✘ 監視開始失敗: ${err}`, "error");
    return;
  }

  // Google絞り込み検索URLを外部ブラウザで開く
  addLog(`ブラウザで検索ページを開きます (${watchSession.targets.length} 個)`, "info");
  for (const t of watchSession.targets) {
    if (t.page_url) {
      await invoke("open_browser", { url: t.page_url }).catch(() => addLog(`ブラウザを開けませんでした: ${t.file_name}`, "error"));
    }
  }

  // モーダルを描画
  renderWatchTargets();
  setWatchPhase("waiting");
  watchStatusMsg.textContent = "";
  watchDoneBtn.style.display = "none";
  watchCancelBtn.style.display = "";
  watchModal.style.display = "flex";

  // GSAP パルスアニメーション (待機中)
  startPulse();
}

function renderWatchTargets() {
  watchTargetList.innerHTML = watchSession.targets.map((t, i) => `
    <div class="watch-target-item" id="watch-target-${i}">
      <span class="dir-badge">${escapeHtml(t.target_dir)}/</span>
      <span class="t-name">${escapeHtml(t.file_name)}</span>
      <span class="t-status" data-status="waiting">${window.i18n?.t("watch.item.waiting") || "待機中"}</span>
    </div>
  `).join("");
}

function setWatchPhase(phase) {
  const label = {
    waiting: window.i18n?.t("watch.phase.waiting") || "待機中",
    hashing: window.i18n?.t("watch.phase.hashing") || "ハッシュ検証中",
    placed: window.i18n?.t("watch.phase.placed") || "配置完了",
    completed: window.i18n?.t("watch.phase.completed") || "回収完了",
  }[phase] || phase;
  watchPhaseBadge.textContent = label;
  watchPhaseBadge.className = "watch-phase-badge " + phase;
}

// GSAP パルス
let pulseAnim = null;
function startPulse() {
  if (typeof gsap === "undefined" || !watchIcon) return;
  pulseAnim?.kill();
  pulseAnim = gsap.to(watchIcon, {
    scale: 1.15, opacity: 0.75, duration: 0.9,
    repeat: -1, yoyo: true, ease: "sine.inOut",
  });
}
function stopPulse() {
  pulseAnim?.kill();
  pulseAnim = null;
  if (watchIcon) gsap.set(watchIcon, { scale: 1, opacity: 1 });
}

// watcher イベント処理
function handleWatcherProgress(ev) {
  if (!watchSession.sessionId || ev.session_id !== watchSession.sessionId) return;
  addLog(ev.message, ev.phase === "placed" || ev.phase === "completed" ? "success" : "info");
  watchStatusMsg.textContent = ev.message;

  if (ev.phase === "detected" || ev.phase === "hashing") {
    setWatchPhase("hashing");
    // 対応するアイテムを検証中に
    if (ev.matched_file) markTarget(ev.matched_file, "hashing");
    else document.querySelectorAll(".watch-target-item:not(.placed)").forEach(el => {
      if (el.querySelector(".t-status")) setItemStatus(el, "hashing");
    });
  }

  if (ev.phase === "placed") {
    setWatchPhase("placed");
    markTarget(ev.matched_file, "placed");
    if (typeof gsap !== "undefined") {
      gsap.fromTo(watchPhaseBadge, { scale: 1.25 }, { scale: 1, duration: 0.4, ease: "back.out(2)" });
    }
  }

  if (ev.phase === "completed") {
    setWatchPhase("completed");
    stopPulse();
    watchDoneBtn.style.display = "";
    watchCancelBtn.style.display = "none";
    watchSession.sessionId = null; // 自動停止済み
  }
}

function setItemStatus(el, state) {
  const st = el.querySelector(".t-status");
  if (!st) return;
  const label = state === "placed"
    ? (window.i18n?.t("watch.item.placed") || "✔ 配置済み")
    : state === "hashing"
      ? (window.i18n?.t("watch.item.hashing") || "検証中...")
      : (window.i18n?.t("watch.item.waiting") || "待機中");
  st.textContent = label;
  st.dataset.status = state;
}

function markTarget(fileName, state) {
  watchSession.targets.forEach((t, i) => {
    if (t.file_name === fileName) {
      const el = document.getElementById(`watch-target-${i}`);
      if (el) {
        setItemStatus(el, state);
        if (state === "placed") el.classList.add("placed");
        if (state === "hashing") el.classList.add("hashing");
      }
    }
  });
}

// 監視先変更
async function changeWatchDir() {
  try {
    const selected = await open({ directory: true, multiple: false, title: "ダウンロードフォルダを選択" });
    if (!selected) return;
    localStorage.setItem("watch_dir", selected);
    await restartWatcher(selected);
  } catch (e) {
    addLog(`フォルダ選択エラー: ${e}`, "error");
  }
}

async function restartWatcher(dir) {
  // 既存セッションを確実に破棄して再開
  if (watchSession.sessionId) {
    await invoke("stop_download_watcher", { sessionId: watchSession.sessionId }).catch(() => {});
    watchSession.sessionId = null;
  }
  if (watchSession.isClosed) return;
  try {
    const res = await invoke("start_download_watcher", {
      targetsJson: JSON.stringify(watchSession.targets.filter(t => !isTargetPlaced(t))),
      watchDir: dir,
      destRoot: watchSession.gamePath,
    });
    watchSession.sessionId = res.session_id;
    watchSession.watchDir = res.watch_dir;
    watchDirPath.textContent = res.watch_dir;
    setWatchPhase("waiting");
    startPulse();
    addLog(`📁 監視先を変更: ${res.watch_dir}`, "info");
  } catch (err) {
    addLog(`✘ 監視再開失敗: ${err}`, "error");
  }
}

function isTargetPlaced(t) {
  return watchSession.targets.find(x => x.file_name === t?.file_name)?.placed === true;
}

// 🔍 ワンショットスキャン (フォールバック)
async function scanNowHandler() {
  if (!watchSession.gamePath) return;
  addLog("🔍 ダウンロードフォルダをスキャン中 (一括ハッシュ照合)...", "info");
  setWatchPhase("hashing");
  try {
    const result = await invoke("scan_download_dir_once", {
      targetsJson: JSON.stringify(watchSession.targets.filter(t => !t.placed)),
      watchDir: watchSession.watchDir,
      destRoot: watchSession.gamePath,
    });
    if (result.matched.length > 0) {
      for (const m of result.matched) {
        m.placed = true;
        markTarget(m.file_name, "placed");
        addLog(`✔ 配置完了: ${m.file_name} (SHA-512 一致)`, "success");
      }
      setWatchPhase("placed");
      checkAllPlaced();
    } else {
      addLog("一致するファイルは見つかりませんでした。ダウンロードが完了してから再度スキャンしてください。", "warning");
      setWatchPhase("waiting");
    }
  } catch (err) {
    addLog(`✘ スキャン失敗: ${err}`, "error");
    setWatchPhase("waiting");
  }
}

// 📂 ファイル直接選択 (フォールバック)
async function pickFileHandler() {
  try {
    const selected = await open({
      directory: false, multiple: true,
      filters: [{ name: "MODファイル", extensions: ["jar", "zip"] }],
      title: "ダウンロード済みのJARファイルを選択"
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    await placeFromPaths(paths);
  } catch (e) {
    addLog(`ファイル選択エラー: ${e}`, "error");
  }
}

async function placeFromPaths(paths) {
  if (!watchSession.gamePath || paths.length === 0) return;
  addLog(`📂 ${paths.length} 個のファイルをハッシュ照合中...`, "info");
  setWatchPhase("hashing");
  try {
    const result = await invoke("place_files_from_paths", {
      paths: paths,
      targetsJson: JSON.stringify(watchSession.targets.filter(t => !t.placed)),
      destRoot: watchSession.gamePath,
    });
    for (const p of result.placed) {
      p.placed = true;
      markTarget(p.file_name, "placed");
      addLog(`✔ 配置完了: ${p.file_name} (SHA-512 一致)`, "success");
    }
    for (const [path, reason] of result.failed) {
      addLog(`✘ ${path}: ${reason}`, "error");
    }
    if (result.placed.length > 0) {
      setWatchPhase("placed");
      checkAllPlaced();
    } else {
      setWatchPhase("waiting");
    }
  } catch (err) {
    addLog(`✘ 配置失敗: ${err}`, "error");
    setWatchPhase("waiting");
  }
}

function checkAllPlaced() {
  const remaining = watchSession.targets.filter(t => !t.placed);
  if (remaining.length === 0) {
    setWatchPhase("completed");
    stopPulse();
    watchDoneBtn.style.display = "";
    watchCancelBtn.style.display = "none";
    if (watchSession.sessionId) {
      invoke("stop_download_watcher", { sessionId: watchSession.sessionId }).catch(() => {});
      watchSession.sessionId = null;
    }
  }
}

// キャンセル → ウォッチャー確実に破棄
async function cancelWatch() {
  if (watchSession.sessionId) {
    try {
      await invoke("stop_download_watcher", { sessionId: watchSession.sessionId });
      addLog("監視を停止・破棄しました。", "info");
    } catch (e) { /* 既に停止済み */ }
    watchSession.sessionId = null;
  }
  closeWatchModal();
}

function closeWatchModal() {
  watchSession.isClosed = true;
  // モーダルを閉じたら監視は必ず停止 (常時バックグラウンド監視は絶対禁止)
  if (watchSession.sessionId) {
    invoke("stop_download_watcher", { sessionId: watchSession.sessionId }).catch(() => {});
    watchSession.sessionId = null;
  }
  stopPulse();
  watchModal.style.display = "none";
  // 手動DLリストを更新 (未配置のみ)
  currentManualTargets = currentManualTargets.filter(m => {
    const t = watchSession.targets.find(x => x.file_name === m.file_name);
    return !(t && t.placed);
  });
  if (manualDlList) {
    if (currentManualTargets.length > 0) {
      manualDlList.innerHTML = currentManualTargets.map(item => `
        <div style="display: flex; align-items: center; gap: 10px; padding: 8px 0; border-bottom: 1px solid var(--glass-border);">
          <span class="dir-badge">${escapeHtml(item.target_dir)}/</span>
          <span class="file-name-cell" style="flex: 1;">${escapeHtml(item.file_name)}</span>
          ${item.page_url ? `<a class="url-link" href="${escapeHtml(item.page_url)}" target="_blank" style="white-space: nowrap;">🔗 ページを開く</a>` : `<span style="color: var(--text-muted); font-size: 12px;">リンクなし</span>`}
        </div>
      `).join("");
    } else {
      manualDlSection.style.display = "none";
    }
  }
  addLog("モーダルを閉じました (監視は自動停止済み)。", "info");
}
