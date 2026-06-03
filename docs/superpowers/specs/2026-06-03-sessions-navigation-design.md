# Sessions 列表導覽優化(搜尋 / 整理狀態 / 主題)設計

> **Goal**: 解決「會議場次太多、一畫面看不完、難找、不知道哪些整理過」的痛點。讓 Sessions 分頁
> 能**搜尋(主題 + 逐字稿內文)**、**依整理狀態篩選**、卡片**顯示主題 + 整理徽章**,並能**手動標記
> 整理完成**,讓整理會議的流程清楚不亂。
>
> **Plan output**: 本 spec → `writing-plans` → `docs/superpowers/plans/2026-06-03-sessions-navigation.md` → 實作。

## 背景(現況,已查證)

- 後端 `main.rs::list_sessions_detailed()`:讀**全部** `~/.mori/meetings/` 場次,各建 `SessionSummary`
  (timeline.json + preview md + 數段),排序回前端。無搜尋 / 篩選 / 分頁。
- `SessionSummary`(`session_store.rs`)欄位:`id / started_at / duration_secs / public_segs /
  internal_segs / preview / corrupt`。**沒有會議主題**(topic 在各場 `meeting-info.json`,沒進 summary)。
- 前端 `SessionsTab.tsx`:把全部 summary render 成 `MeetingCard`;點卡 → `SessionWorkspace` 編輯。
  無搜尋框 / 篩選 / 狀態指示。
- 結果:62 場(且持續增長)→ 卡片牆、看不完、只能用日期 + 預覽肉眼找,且沒有「主題」可看、
  分不出哪些整理過。

## 範圍(yazelin 2026-06-03 拍板)

| # | 決議 | 值 |
|---|---|---|
| 1 | 整理狀態判定 | **手動標記完成**:使用者按「整理完成」→ 存旗標;非自動推導。 |
| 2 | 搜尋範圍 | **主題 + 日期(排序)+ 狀態 + 逐字稿內文全文**。 |
| 3 | 做法 | A:metadata 篩選走前端(即時),逐字稿全文搜尋走後端命令(較重、需要時才跑)。 |
| 4 | 卡片 | 顯示**會議主題** + **整理狀態徽章**(已整理 / 未整理)+「標記完成 / 取消」鈕。 |
| 5 | 進入編輯 | 維持點卡 → `SessionWorkspace`(找得到就進得去,不另做)。 |

## 做法選擇

**採 A:前端篩選 + 後端全文搜尋分工。**
- 主題 / 狀態篩選對「已載入的清單」在前端即時做(打字不 round-trip、不頓)。
- 逐字稿內文搜尋較重(要讀各場檔案)→ 後端命令,只在使用者開「含逐字稿內文」時 debounce 觸發。

**否決 B(全部塞後端一個 query 命令)**:每次打字都 round-trip → 頓;且前端篩選彈性差。

## 架構

### 後端 `src-tauri/src/session_store.rs`(SessionSummary 擴充 + 讀 meeting-info)

- `SessionSummary` 加兩欄:`#[serde(default)] topic: String`、`#[serde(default)] organized: bool`。
- `read_session_summary` 讀該場 `meeting-info.json`(已有,存 topic/participants)取 `topic`;
  另讀**獨立** `session-state.json`(`{ "organized": bool }`)取 `organized`。
  **organized 分檔存、不放 meeting-info.json** —— 因為既有 `set_meeting_info`/`set_meeting_info_for`
  以 `{topic,participants}` **覆寫整個** meeting-info.json,放一起會被洗掉;分檔互不干擾、也免動既有 writers。
  缺檔/缺欄 → topic 空 / organized false(graceful,不視為 corrupt)。

### 後端 `src-tauri/src/main.rs`(兩個新命令 + 註冊)

- `set_session_organized(session_id: String, organized: bool) -> Result<(), String>`:
  寫該場獨立 `session-state.json`(`{ "organized": bool }`,原子寫:tmp+rename 或直接 write)。
  **分檔故完全不碰 meeting-info.json / 既有 writers**(零回歸風險)。
- `search_sessions_fulltext(query: String) -> Vec<String>`:
  query 去空白後為空 → 回空。否則掃每場匯出的逐字稿 md(`meeting.md` 或 `meeting.public.md` /
  `meeting.internal.md`,存在哪讀哪),**case-insensitive 子字串**命中 → 收該 session_id。
  某場讀檔失敗 → 跳過不中斷。回命中的 session_id 清單。
- 兩個命令註冊進 `generate_handler!`。

### 前端 `src/tabs/SessionsTab.tsx`(搜尋/篩選列 + 狀態)

- 型別 `SessionSummary` 加 `topic: string`、`organized: boolean`。
- 頂部加**搜尋/篩選列**(沿用 recorder 既有 inline style + token):
  - 搜尋框 `query`:即時對已載入清單依 **topic 子字串**(case-insensitive)過濾。
  - **「含逐字稿內文」checkbox**:開啟時,`query` 變動 debounce(~300ms)呼 `search_sessions_fulltext(query)`
    → 得 `matchedIds: Set` → 結果 = 主題命中 ∪ 內文命中(id 在 matchedIds)。關閉時只走主題。
  - **狀態 chips**:全部 / 已整理 / 未整理 → 即時依 `organized` 過濾。
  - 預設排序日期新→舊(現狀不變)。
- 篩選後只 render 命中的卡片 → 解「一畫面看不完」。

### 前端 `src/components/MeetingCard.tsx`(顯示主題 + 狀態 + 標記鈕)

- 顯示 `summary.topic`(空則顯示「(未命名會議)」或日期)。
- **狀態徽章**:`organized` → 「已整理 ✓」(綠/found token);否則「未整理」(dim)。
- **「標記完成 / 取消」鈕**:呼 `set_session_organized(id, !organized)` → 樂觀更新 + 父層刷新該場狀態。
- 沿用既有 MeetingCard 樣式 token(不寫死色值)。

## 資料流

```
開 Sessions 分頁 → list_sessions_detailed()(含 topic/organized) → 卡片(顯示主題+徽章)
搜尋:
  打字 → 前端依 topic 即時過濾
  勾「含逐字稿內文」→ debounce search_sessions_fulltext(query) → matchedIds → 併入
狀態 chips → 前端依 organized 過濾
標記完成 → set_session_organized(id, bool) → 寫 session-state.json → 卡片徽章更新
```

## 錯誤處理

- `meeting-info.json` 缺 / 壞 → topic 空;`session-state.json` 缺 / 壞 → organized false(graceful;不視為 corrupt)。
- organized 與 topic/participants **分檔**,彼此寫入互不覆蓋(免動既有 meeting-info writers)。
- `search_sessions_fulltext` 某場讀檔失敗 → 跳過該場;query 空 → 回空清單(前端視為「不限制內文」)。
- 全文搜尋是 O(場數 × md 大小):62 場沒問題;**大量場次的效能 / 虛擬化列 follow-up**(log 不靜默,搜尋慢時可加提示)。

## 測試(TDD)

- `set_session_organized` round-trip:寫 organized=true → 再讀 summary 得 organized=true;且該場 `meeting-info.json` 的 topic **不受影響**(分檔;先放含 topic 的 meeting-info.json → set_organized → 驗 summary 同時有 topic + organized)。
- `search_sessions_fulltext`:temp 建 2-3 場(各放 meeting.md / public.md 含不同字)→ query 命中對的場、不命中的不回、空 query 回空、case-insensitive。
- `SessionSummary` 含 topic / organized(serde default：舊 meeting-info.json 無 organized → false)。
- **回歸**:既有 SessionSummary 欄位 / 排序 / corrupt 行為不變。
- 前端:沿用 recorder 既有前端慣例(手測為主)。
- `bash scripts/verify.sh` 全綠。
- 真機手測:Sessions 分頁出現搜尋列;打主題關鍵字即時縮清單;勾「含逐字稿內文」搜得到內文命中場次;狀態 chips 篩已整理/未整理;按「標記完成」徽章變、重開分頁仍記得;卡片看得到主題。

## 非目標 / Follow-up

- **虛擬化 / 分頁**(數百場才需要;本次靠篩選縮 render)。
- 日期範圍精細篩選 UI、搜尋命中**高亮**、tag / 自訂分類。
- 全文搜尋索引(目前每次掃檔;量大再說)。
- 「已整理」自動推導(本次純手動旗標)。

## 驗證

- `bash scripts/verify.sh` 全綠。
- 真機手測清單(見上),逐項通過(動了 Rust → 重啟 tauri dev)。
