# 摘要設定 UI(Summary settings)設計

> **Goal**: 在 Settings 分頁加「摘要 / Summary」段,讓使用者在 app 內設定:Groq 模型、Ollama 模型、
> Ollama base_url、強制本機 toggle(寫 recorder 自己 config),以及 **Groq API key**(寫共享
> `~/.mori/config.json`)。補掉「摘要相關全靠手改 config 檔、沒有 UI」的缺口。
>
> **Plan output**: 本 spec → `writing-plans` → `docs/superpowers/plans/2026-06-03-summary-settings-ui.md` → 實作。

## 背景(現況,已查證)

- Settings 分頁(`src/tabs/SettingsTab.tsx`)目前只有**轉錄**設定:whisper 模型(small/large-v3-turbo)、
  語言、繁體、VAD 參數。**沒有**摘要模型 / Groq / Ollama / API key 任何 UI。
- 摘要模型欄位**已存在** `RecorderConfig`(`config.rs`):`summary_groq_model`(預設 `openai/gpt-oss-120b`)、
  `summary_ollama_model`(預設 `qwen3:4b-instruct-2507-q4_K_M`)、`summary_ollama_base_url`
  (預設 `http://localhost:11434`)、`summary_force_local_default`(bool)。但**無 UI**,只能手改
  `~/.mori/meeting-recorder/config.json`。
- **Groq API key 不在** recorder config。`summarize.rs::resolve_groq_api_key`(:453)2 段解析:
  ① env `GROQ_API_KEY` ② **共享** `~/.mori/config.json` 的 JSON pointer `/providers/groq/api_key`
  (helper:`is_placeholder` :429、`read_json_pointer` :435、shared-config path :445-448)。無 UI。

## 範圍(yazelin 2026-06-03 拍板)

| # | 決議 | 值 |
|---|---|---|
| 1 | 摘要模型欄位 | Groq 模型 / Ollama 模型 / Ollama base_url / 強制本機 toggle → 走既有 `get_config`/`set_config`(recorder config)。 |
| 2 | Groq API key 存哪 | **寫共享 `~/.mori/config.json` 的 `/providers/groq/api_key`**(read-modify-write,保留其他欄位)。與摘要 code 的讀取路徑一致。 |
| 3 | key 顯示 | **password 遮罩**,載入時顯示「已設定 ●●● / 未設定」,**不回填明碼**;輸入新值才寫。 |
| 4 | UI 位置 | Settings 分頁新增「摘要 / Summary」段,沿用既有 setting-field 樣式。 |

## 做法

摘要模型四欄已在 RecorderConfig → **不加後端**,前端補欄位即可(set_config 送完整 cfg)。
Groq API key 因在共享檔、且要遮罩 → 加 **2 個薄命令**(讀狀態 / 寫值),複用 summarize.rs 既有 helper。

## 架構

### 後端 `src-tauri/src/summarize.rs`(共享-config helper 可能需 pub 化)

- 既有私有的 shared-config-path fn(`~/.mori/config.json`)+ `is_placeholder` + `read_json_pointer`
  目前供 `resolve_groq_api_key` 用。本案需要:
  - **讀狀態**:`groq_api_key_present(config_path) -> bool` —— `read_json_pointer(path, "/providers/groq/api_key")`
    回 `Some(非 placeholder 非空)` 即 true。純函式、可測。
  - **寫值**:`set_groq_api_key_at(config_path, key) -> Result<(), String>` —— read-modify-write:
    讀現有 JSON(缺檔 → `{}` 起手;**壞檔 parse 失敗 → 回 Err、不覆寫**,避免洗掉共享檔裡其他 app 的設定)
    → 確保 `providers.groq` 物件存在 → 設 `api_key = key`(空字串則移除該 key)→ pretty 寫回。
    **保留 `providers` 下其他 provider、頂層其他欄位**。純函式(吃 path)、可測。
  - 把目前算 `~/.mori/config.json` 路徑的 helper 提成 `pub fn shared_config_path() -> Option<PathBuf>`(若尚未 pub)。

### 後端 `src-tauri/src/main.rs`(2 個命令 + 註冊)

- `groq_key_status() -> bool`:`summarize::shared_config_path()` → `groq_api_key_present(&path)`(缺路徑 → false)。
- `set_groq_api_key(key: String) -> Result<(), String>`:`shared_config_path()` → `set_groq_api_key_at(&path, &key)`(缺路徑 → Err)。
- 兩者註冊進 `generate_handler!`。

### 前端 `src/tabs/SettingsTab.tsx`(新「摘要」段)

- `RecorderConfig` TS 型別補四欄:`summary_groq_model: string` / `summary_ollama_model: string` /
  `summary_ollama_base_url: string` / `summary_force_local_default: boolean`(`DEFAULTS` 也補)。
  (runtime 物件本來就有這些欄、`setCfg({...cfg, …})` spread 保留;補型別是為了型別安全 + 新輸入。)
- 在既有轉錄設定之後加「摘要 / Summary」段:
  - Groq 模型(text input,綁 `summary_groq_model`)。
  - Ollama 模型(text,綁 `summary_ollama_model`)。
  - Ollama base_url(text,綁 `summary_ollama_base_url`)。
  - **強制本機**(checkbox,綁 `summary_force_local_default`)。
  - 以上走既有「儲存」按鈕(`set_config`,送完整 cfg)。
- **Groq API key**(獨立小區塊,不走 set_config):
  - `useEffect` 載入呼 `groq_key_status` → state `keySet`;顯示「已設定 ●●●」或「未設定」。
  - password `<input>`(state `keyInput`,初始空)+「儲存金鑰」按鈕 → `invoke("set_groq_api_key",{key:keyInput})`
    → 成功後 `keySet=true`、清空 keyInput、顯示「已更新」。
  - 提示文字:此金鑰寫入**共享** `~/.mori/config.json`,宇宙其他 app 共用。

## 資料流

```
載入 Settings → get_config(含 summary 四欄)+ groq_key_status() → 表單 + 「已設定/未設定」
改摘要四欄 → 「儲存」→ set_config(完整 cfg) → ~/.mori/meeting-recorder/config.json
輸入 key → 「儲存金鑰」→ set_groq_api_key(key) → read-modify-write 共享 ~/.mori/config.json /providers/groq/api_key
```

## 錯誤處理

- 共享 config 缺 → `groq_key_status` false、`set_groq_api_key` 建 `{}` 再寫;**壞檔(parse 失敗)→ `set_groq_api_key` 回 Err、不覆寫**(共享檔含其他 app 設定,不可因寫 groq key 而清掉;由 user 手動修檔)。`groq_key_status` 對壞檔回 false。
- `set_groq_api_key_at` **read-modify-write**:必須保留 `providers` 下其他 provider、頂層其他欄位,不可整檔覆寫成只有 groq。
- 空字串 key → 視為清除(移除 `/providers/groq/api_key` 或設空字串)。
- `set_config` 送完整 cfg(spread 保留 summary 欄)→ 不會洗掉摘要欄位(回歸點)。

## 測試(TDD)

- `groq_api_key_present`:temp config 有合法 key → true;placeholder / 空 / 缺 pointer / 缺檔 → false。
- `set_groq_api_key_at` round-trip:寫 key → `groq_api_key_present` true;**且預先放的 `/providers/other/x` 與頂層 `foo` 欄位仍在**(不被洗);空字串 → present 變 false;缺檔 → 建檔成功。
- 前端:沿用 recorder 既有前端慣例(手測為主)。
- `bash scripts/verify.sh` 全綠。
- 真機手測:Settings「摘要」段改 Groq/Ollama 模型 + base_url + 強制本機 → 儲存 → 重開仍在(`meeting-recorder/config.json`);key 欄位輸入 → 儲存金鑰 → 顯示「已設定」、`~/.mori/config.json` 出現 `/providers/groq/api_key`、且該檔原有其他內容沒被洗;清空 key 儲存 → 變未設定。

## 非目標 / Follow-up

- 其他 provider(只 Groq);key **連線測試**(打 API 驗證有效);per-session 模型覆寫;Ollama 模型清單下拉(目前純 text 輸入)。

## 驗證

- `bash scripts/verify.sh` 全綠。
- 真機手測清單(見上),逐項通過(動了 Rust → 重啟 tauri dev)。
