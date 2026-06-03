# 摘要設定 UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Settings 分頁加「摘要」段,讓使用者在 app 內設 Groq/Ollama 摘要模型 + base_url + 強制本機(recorder config),以及 Groq API key(寫共享 `~/.mori/config.json`)。

**Architecture:** 摘要模型四欄已在 RecorderConfig → 前端走既有 get_config/set_config。Groq key 在共享檔 + 要遮罩 → 加可測核心(`groq_api_key_present` / `set_groq_api_key_at`,summarize.rs)+ 2 薄命令(main.rs)。前端在 SettingsTab 加摘要段 + key 區塊。

**Tech Stack:** Rust(serde_json / dirs)、Tauri v2 command、React + TS、react-i18next。

**Spec:** `docs/superpowers/specs/2026-06-03-summary-settings-ui-design.md`

**Worktree / branch:** `/home/ct/mori-universe/.worktrees/recorder-summary-settings` @ `feat/summary-settings-ui`(off origin/main `811ab0b`)。

⚠ cargo 在 `src-tauri/` 內跑;先 `npm run build`(generate_context 需 dist)再 cargo。手測 `npm run tauri dev`(動 Rust 要重啟)。

---

## File Structure

| 檔案 | 動作 | 責任 |
|---|---|---|
| `src-tauri/src/summarize.rs` | Modify | `mori_config_path` pub 化 + `groq_api_key_present` + `set_groq_api_key_at`(read-modify-write,壞檔→Err)+ 測試 |
| `src-tauri/src/main.rs` | Modify | `groq_key_status` / `set_groq_api_key` 命令 + 註冊 |
| `src/i18n/locales/{en,zh-TW}.json` | Modify | `settings.summary_*` + groq key 字串 |
| `src/tabs/SettingsTab.tsx` | Modify | RecorderConfig 型別+DEFAULTS 補 summary 四欄;摘要段 4 欄;Groq key 區塊 |

---

## Task 1: summarize.rs — groq key 讀/寫核心

**Files:** Modify `src-tauri/src/summarize.rs`

- [ ] **Step 1: 寫失敗測試(tests 模組內)**

```rust
    #[test]
    fn groq_api_key_present_detects_set_and_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.json");
        assert!(!groq_api_key_present(&p)); // 缺檔
        std::fs::write(&p, r#"{"providers":{"groq":{"api_key":"gsk_real"}}}"#).unwrap();
        assert!(groq_api_key_present(&p));
        std::fs::write(&p, r#"{"providers":{"groq":{"api_key":"YOUR_GROQ_KEY"}}}"#).unwrap();
        assert!(!groq_api_key_present(&p)); // placeholder
        std::fs::write(&p, r#"{"providers":{"groq":{"api_key":""}}}"#).unwrap();
        assert!(!groq_api_key_present(&p)); // 空
    }

    #[test]
    fn set_groq_api_key_at_roundtrips_and_preserves_other_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.json");
        // 既有共享檔:含其他 provider + 頂層欄位
        std::fs::write(&p, r#"{"foo":"bar","providers":{"openai":{"api_key":"oa"}}}"#).unwrap();
        set_groq_api_key_at(&p, "gsk_new").unwrap();
        assert!(groq_api_key_present(&p));
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v.pointer("/providers/groq/api_key").unwrap(), "gsk_new");
        assert_eq!(v.pointer("/providers/openai/api_key").unwrap(), "oa"); // 其他 provider 保留
        assert_eq!(v.pointer("/foo").unwrap(), "bar"); // 頂層保留
    }

    #[test]
    fn set_groq_api_key_at_creates_when_missing_and_clears_on_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sub/config.json"); // 連目錄都不存在
        set_groq_api_key_at(&p, "gsk_x").unwrap();
        assert!(groq_api_key_present(&p));
        set_groq_api_key_at(&p, "").unwrap(); // 空字串 = 清除
        assert!(!groq_api_key_present(&p));
    }

    #[test]
    fn set_groq_api_key_at_refuses_to_clobber_corrupt_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.json");
        std::fs::write(&p, "{ this is not json").unwrap();
        let r = set_groq_api_key_at(&p, "gsk_x");
        assert!(r.is_err());
        // 壞檔原樣保留(沒被覆寫)
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ this is not json");
    }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-summary-settings/src-tauri && cargo test groq_api_key 2>&1 | tail -15`
Expected: 編譯失敗(`groq_api_key_present` / `set_groq_api_key_at` 不存在)。

- [ ] **Step 3: mori_config_path pub 化**

`summarize.rs:447` 把 `fn mori_config_path()` 改成 `pub fn mori_config_path()`:
```rust
pub fn mori_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".mori").join("config.json"))
}
```

- [ ] **Step 4: 加 groq_api_key_present + set_groq_api_key_at**

在 `resolve_groq_api_key_at`(`:458-470`)之後加:
```rust
/// 共享 config 的 /providers/groq/api_key 是否已設(非空非 placeholder)。
pub fn groq_api_key_present(config_path: &Path) -> bool {
    read_json_pointer(config_path, "/providers/groq/api_key").is_some()
}

/// read-modify-write 共享 config 的 /providers/groq/api_key。
/// 缺檔 → 建 {}(連同父目錄);壞檔(parse 失敗)→ Err,**不覆寫**(共享檔含其他 app 設定);
/// 空 key → 移除該欄;保留 providers 下其他 provider 與頂層其他欄位。
pub fn set_groq_api_key_at(config_path: &Path, key: &str) -> Result<(), String> {
    let mut root: serde_json::Value = if config_path.exists() {
        let text = std::fs::read_to_string(config_path)
            .map_err(|e| format!("read shared config: {e}"))?;
        if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&text).map_err(|e| {
                format!("shared config 不是合法 JSON,拒絕覆寫(請手動修 {}): {e}", config_path.display())
            })?
        }
    } else {
        serde_json::json!({})
    };
    let obj = root.as_object_mut().ok_or("shared config 頂層不是 JSON object,拒絕覆寫")?;
    let providers = obj.entry("providers").or_insert_with(|| serde_json::json!({}));
    let providers = providers.as_object_mut().ok_or("shared config /providers 不是 object")?;
    let groq = providers.entry("groq").or_insert_with(|| serde_json::json!({}));
    let groq = groq.as_object_mut().ok_or("shared config /providers/groq 不是 object")?;
    if key.trim().is_empty() {
        groq.remove("api_key");
    } else {
        groq.insert("api_key".into(), serde_json::Value::String(key.to_string()));
    }
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir shared config dir: {e}"))?;
    }
    let body = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(config_path, body).map_err(|e| format!("write shared config: {e}"))
}
```

- [ ] **Step 5: 跑測試確認通過 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-summary-settings/src-tauri && cargo test groq_api_key 2>&1 | tail -15`
Expected: 4 個新測試 PASS;既有 summarize 測試仍綠。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-summary-settings
git add src-tauri/src/summarize.rs
git commit -m "feat(recorder): groq_api_key_present + set_groq_api_key_at(共享 config read-modify-write,壞檔不覆寫)"
```

---

## Task 2: main.rs — groq_key_status / set_groq_api_key 命令

**Files:** Modify `src-tauri/src/main.rs`（命令加在摘要相關區或 list_sessions_detailed 附近;註冊在 `generate_handler!` `:913`)

- [ ] **Step 1: 加兩個命令**

```rust
/// 共享 ~/.mori/config.json 是否已設 Groq API key(給 Settings UI 顯示「已設定/未設定」;不回傳 key)。
#[tauri::command]
fn groq_key_status() -> bool {
    summarize::mori_config_path()
        .map(|p| summarize::groq_api_key_present(&p))
        .unwrap_or(false)
}

/// 設定 Groq API key 到共享 ~/.mori/config.json(空字串 = 清除)。
#[tauri::command]
fn set_groq_api_key(key: String) -> Result<(), String> {
    let path = summarize::mori_config_path().ok_or("無法解析 ~/.mori/config.json 路徑")?;
    summarize::set_groq_api_key_at(&path, &key)
}
```

- [ ] **Step 2: 註冊進 generate_handler!**

在 `generate_handler!` 清單加:
```rust
            groq_key_status,
            set_groq_api_key,
```

- [ ] **Step 3: 編譯 + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-summary-settings/src-tauri && cargo check 2>&1 | tail -6`
Expected: 通過。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-summary-settings
git add src-tauri/src/main.rs
git commit -m "feat(recorder): groq_key_status + set_groq_api_key 命令 + 註冊"
```

---

## Task 3: i18n 摘要設定字串(en + zh-TW)

**Files:** Modify `src/i18n/locales/en.json`、`src/i18n/locales/zh-TW.json`（`settings` 物件)

- [ ] **Step 1: en.json — settings 物件加鍵**

在 `settings` 物件內(任一既有鍵後,維持 JSON 合法)加:
```json
    "summary_section": "Summary",
    "summary_groq_model": "Groq summary model",
    "summary_ollama_model": "Local Ollama model",
    "summary_ollama_base_url": "Ollama base URL",
    "summary_force_local": "Force local only",
    "summary_force_local_hint": "Never call cloud Groq — use local Ollama only.",
    "groq_api_key": "Groq API key",
    "groq_key_set": "Set ●●●",
    "groq_key_unset": "Not set",
    "groq_key_hint": "Written to shared ~/.mori/config.json (shared across Mori apps).",
    "save_key": "Save key",
    "key_saved": "Updated",
```

- [ ] **Step 2: zh-TW.json — 對稱鍵**

```json
    "summary_section": "摘要",
    "summary_groq_model": "Groq 摘要模型",
    "summary_ollama_model": "本機 Ollama 模型",
    "summary_ollama_base_url": "Ollama 位址",
    "summary_force_local": "強制只用本機",
    "summary_force_local_hint": "不呼叫雲端 Groq,只用本機 Ollama。",
    "groq_api_key": "Groq API 金鑰",
    "groq_key_set": "已設定 ●●●",
    "groq_key_unset": "未設定",
    "groq_key_hint": "寫入共享 ~/.mori/config.json,宇宙其他 app 共用。",
    "save_key": "儲存金鑰",
    "key_saved": "已更新",
```

- [ ] **Step 3: 驗 JSON + commit**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-summary-settings && node -e "JSON.parse(require('fs').readFileSync('src/i18n/locales/en.json','utf8'));JSON.parse(require('fs').readFileSync('src/i18n/locales/zh-TW.json','utf8'));console.log('json ok')"`
Expected: `json ok`。
```bash
cd /home/ct/mori-universe/.worktrees/recorder-summary-settings
git add src/i18n/locales/en.json src/i18n/locales/zh-TW.json
git commit -m "feat(recorder): i18n 摘要設定字串(en + zh-TW)"
```

---

## Task 4: SettingsTab 摘要段 + Groq key 區塊

**Files:** Modify `src/tabs/SettingsTab.tsx`

- [ ] **Step 1: RecorderConfig 型別 + DEFAULTS 補 summary 四欄**

`SettingsTab.tsx` 的 `interface RecorderConfig`(`:11-19`)在 `model: string;` 後加:
```tsx
  summary_groq_model: string;
  summary_ollama_model: string;
  summary_ollama_base_url: string;
  summary_force_local_default: boolean;
```
`DEFAULTS`(`:21-29`)在 `model: "small",` 後加(值對齊 config.rs 預設):
```tsx
  summary_groq_model: "openai/gpt-oss-120b",
  summary_ollama_model: "qwen3:4b-instruct-2507-q4_K_M",
  summary_ollama_base_url: "http://localhost:11434",
  summary_force_local_default: false,
```
(⚠ DEFAULTS 也是「重設」鈕用的,沒補的話重設→儲存會洗掉摘要欄。)

- [ ] **Step 2: 加 Groq key 狀態 state + 載入**

在 `const [saved, setSaved] = useState(false);`(`:34`)後加:
```tsx
  const [keySet, setKeySet] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [keySaved, setKeySaved] = useState(false);
  useEffect(() => {
    invoke<boolean>("groq_key_status").then(setKeySet).catch(() => setKeySet(false));
  }, []);
  const saveKey = async () => {
    try {
      await invoke("set_groq_api_key", { key: keyInput });
      setKeySet(keyInput.trim() !== "");
      setKeyInput("");
      setKeySaved(true);
      setTimeout(() => setKeySaved(false), 2000);
    } catch (e) { console.error(e); }
  };
```

- [ ] **Step 3: 加「摘要」段(VAD SettingField 之後、儲存鈕 `:124` 之前)**

在 `max_segment` 的 `<SettingField … />`(結束於 `:123`)之後插入:
```tsx
      <h4>{t("settings.summary_section")}</h4>
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.summary_groq_model")}</span>
          <input className="setting-field-input" type="text" value={cfg.summary_groq_model}
            onChange={(e) => setCfg({ ...cfg, summary_groq_model: e.target.value })} />
        </div>
      </div>
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.summary_ollama_model")}</span>
          <input className="setting-field-input" type="text" value={cfg.summary_ollama_model}
            onChange={(e) => setCfg({ ...cfg, summary_ollama_model: e.target.value })} />
        </div>
      </div>
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.summary_ollama_base_url")}</span>
          <input className="setting-field-input" type="text" value={cfg.summary_ollama_base_url}
            onChange={(e) => setCfg({ ...cfg, summary_ollama_base_url: e.target.value })} />
        </div>
      </div>
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.summary_force_local")}</span>
          <input type="checkbox" className="setting-field-checkbox"
            checked={cfg.summary_force_local_default}
            onChange={(e) => setCfg({ ...cfg, summary_force_local_default: e.target.checked })} />
        </div>
        <div className="setting-field-hint">{t("settings.summary_force_local_hint")}</div>
      </div>
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.groq_api_key")}</span>
          <span style={{ fontSize: 11, color: keySet ? "var(--found-color)" : "var(--text-dim)" }}>
            {keySet ? t("settings.groq_key_set") : t("settings.groq_key_unset")}
          </span>
        </div>
        <div className="setting-field-row" style={{ gap: 8, marginTop: 4 }}>
          <input className="setting-field-input" type="password" value={keyInput}
            placeholder={keySet ? "••••••" : "gsk_..."}
            onChange={(e) => setKeyInput(e.target.value)} style={{ flex: 1 }} />
          <button className="mmr-btn" onClick={saveKey} disabled={keyInput.trim() === ""}>{t("settings.save_key")}</button>
          {keySaved && <span style={{ color: "var(--found-color)", fontSize: 11 }}>{t("settings.key_saved")}</span>}
        </div>
        <div className="setting-field-hint">{t("settings.groq_key_hint")}</div>
      </div>
```
- [ ] **Step 3b: 在 `src/theme.css` 補 `.setting-field-input`**(SettingsTab 原本沒有 text input,此 class 未定義;沿用既有 token)

在 theme.css 末尾(或 `.setting-field` 規則附近)append:
```css
.setting-field-input {
  flex: 1;
  min-width: 0;
  font-size: calc(12px * var(--scale));
  padding: 4px 8px;
  background: var(--btn-bg);
  color: var(--text);
  border: 0.5px solid var(--border);
  border-radius: 8px;
}
```

- [ ] **Step 4: build 確認**

Run: `cd /home/ct/mori-universe/.worktrees/recorder-summary-settings && npm run build 2>&1 | tail -5`
Expected: tsc/vite 無錯。

- [ ] **Step 5: Commit**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-summary-settings
git add src/tabs/SettingsTab.tsx src/theme.css
git commit -m "feat(recorder): SettingsTab 摘要段(Groq/Ollama 模型+force-local)+ Groq API key 欄位"
```

---

## Task 5: 全量驗證 + 真機手測 + PR

- [ ] **Step 1: verify.sh 全綠**

Run:
```bash
cd /home/ct/mori-universe/.worktrees/recorder-summary-settings
npm run build && bash scripts/verify.sh 2>&1 | tail -20
```
Expected: cargo test 全 PASS(含 groq_api_key 4 新測 + 既有回歸)、npm build、cargo check 乾淨。

- [ ] **Step 2: 真機手測(`npm run tauri dev`)**

- Settings 分頁出現「摘要」段:Groq 模型 / Ollama 模型 / Ollama base_url(text)+ 強制本機(checkbox)。
- 改某欄 → 儲存 → 重開 app 仍在(`~/.mori/meeting-recorder/config.json`)。
- Groq API key:初始顯示「未設定」(或已設定);輸入 key → 儲存金鑰 → 顯示「已設定 ●●●」+「已更新」;`~/.mori/config.json` 出現 `/providers/groq/api_key`,**且該檔原有其他內容沒被洗**。
- 清空 key 框、儲存金鑰 → 變「未設定」。
- 重設鈕 → 摘要欄回預設、其他轉錄欄也回預設(摘要欄沒被漏掉)。

- [ ] **Step 3: push + PR(auto-merge)**

```bash
cd /home/ct/mori-universe/.worktrees/recorder-summary-settings
git push -u origin feat/summary-settings-ui
gh pr create --fill --base main --head feat/summary-settings-ui
gh pr merge --auto --squash
```

- [ ] **Step 4: worktree 清理(merge 後)**

```bash
cd /home/ct/mori-universe/mori-meeting-recorder
git worktree remove /home/ct/mori-universe/.worktrees/recorder-summary-settings
```

---

## Self-Review

**Spec coverage**:
- 範圍#1 摘要四欄走 get_config/set_config → Task 4 Step 1(型別+DEFAULTS)+ Step 3(4 欄 UI,既有 save)。✅
- 範圍#2 key 寫共享 config(read-modify-write 保留)→ Task 1 `set_groq_api_key_at` + Task 2 命令。✅
- 範圍#3 key 遮罩 + 已設定/未設定、不回填 → Task 4 Step 2/3(password + keySet 狀態 + 不回填明碼)。✅
- 範圍#4 Settings 摘要段 → Task 4 Step 3。✅
- 壞檔→Err 不覆寫 → Task 1 `set_groq_api_key_at` + 測試 `refuses_to_clobber_corrupt_file`。✅
- 空字串清除 / 缺檔建立 → Task 1 測試 `creates_when_missing_and_clears_on_empty`。✅
- DEFAULTS 補摘要欄(重設不洗)→ Task 4 Step 1 明標。✅
- 回歸(set_config 送完整 cfg、既有轉錄設定不變)→ Task 4 只加欄不動既有;既有測試仍綠。✅

**Placeholder scan:** 無 TBD/TODO;每 code step 完整 code。`.setting-field-input` 缺 class 註明非阻塞(build 不失敗)。✅

**Type consistency:**
- `groq_api_key_present(&Path)->bool` / `set_groq_api_key_at(&Path,&str)->Result`(Task 1)= main.rs 命令呼叫(Task 2)一致;`mori_config_path` pub 化後 Task 2 用 `summarize::mori_config_path()`。✅
- 命令 `groq_key_status()->bool` / `set_groq_api_key(key)`(Task 2)= 前端 `invoke("groq_key_status")` / `invoke("set_groq_api_key",{key})`(Task 4)一致(key 單字,camelCase 無影響)。✅
- RecorderConfig summary 四欄(Task 4 型別)= config.rs 既有欄位名(`summary_groq_model`/`summary_ollama_model`/`summary_ollama_base_url`/`summary_force_local_default`)逐字一致。✅
- i18n 鍵(Task 3)= Task 4 引用一致。✅
