# Diarization 人工修正工具 設計

- 日期:2026-05-30
- 狀態:設計已 co-review(brainstorming §1/§2 通過),待寫 plan
- 範圍:會後處理工作區裡「手動修正分人結果」的小工具。**不含**聲紋註冊/認人(另開大 spec)。

## 1. 背景與目標

Diarization 是**無監督聲紋分群**,必然不完美 —— 最常見是**過度切**(同一個人散成講者1/3/5,主因:短句聲紋不穩 + 自動門檻過切),偶爾**單段貼錯人**。沒有「認人」之前,需要一層**手動修正**當安全網。目標:在工作區能快速把分錯的講者收乾淨。

修正後文字一樣不重轉(沿用既有 jsonl 主稿 + speakers.json)。

## 2. 已拍板決策

| 主題 | 決策 |
|------|------|
| 合併講者 | 選多個講者 id → 併成一個(reassign 段 + 移除多餘列) |
| 逐段改講者 | 單段重新指派講者;**只能選現有講者**(不在此建新講者) |
| 重跑分人 | **跑前警告「名字會重置」**(方案 B);**名字時間重疊轉移(A)先不做** —— 有了合併,過切直接合併即可,少用重跑,A 複雜度不划算 |
| 不重轉 | 只改 segment 的 `speaker` 欄 + 動 `speakers.json`,文字不動 |

## 3. 架構

### 3.1 純函式(可單測,放 `postprocess.rs`)
- `relabel_merge(segs: Vec<Segment>, keep_id: &str, merge_ids: &[String]) -> Vec<Segment>`:把 `speaker ∈ merge_ids` 的段改成 `keep_id`,其餘不動。
- `relabel_one(segs: Vec<Segment>, seg_id: &str, speaker_id: &str) -> Vec<Segment>`:把 `id == seg_id` 的段 `speaker` 設成 `speaker_id`。
- `drop_speakers(list: Vec<SpeakerInfo>, drop_ids: &[String]) -> Vec<SpeakerInfo>`:濾掉 `drop_ids`。

### 3.2 Tauri commands(`main.rs`,薄包;每軌讀 jsonl → 純函式 → `write_segments_jsonl` 原子寫回)
- `merge_speakers(session_id, keep_id: String, merge_ids: Vec<String>) -> Result<(),String>`:
  對 system + mic-internal **兩軌**各 `read_segments_jsonl → relabel_merge → write_segments_jsonl`(merge_ids 的段可能落在任一軌);再 `read_speakers → drop_speakers(merge_ids) → write_speakers`。`keep_id` 留著(其 speakers.json 列不動;它的 `track` 欄位純顯示,跨軌合併時不糾結)。
- `set_segment_speaker(session_id, track: String, seg_id: String, speaker_id: String) -> Result<(),String>`:
  只動指定 `track` 的 jsonl:`read → relabel_one(seg_id, speaker_id) → write`。(`track + seg_id` 定位,因 seg_id 軌內才唯一。)

### 3.3 前端(`SessionWorkspace.tsx`)
- **講者清單**:每列加勾選框 + 一個「合併所選」鈕。合併規則:所選中第一個(或 UI 標記的)當 `keep_id`,其餘為 `merge_ids` → `merge_speakers` → 重載 speakers + transcript。
- **逐字稿**:每段前面加一個小講者下拉(custom `Select`,列現有講者顯示名)→ 改 → `set_segment_speaker` → 重載。
- **重跑分人**:已分過的 session 再按「分人」→ 先跳確認「會重新分群,已改的講者名字會重置」,確認才跑。

### 3.4 資料模型
**無 schema 變更**。重用 `Segment.speaker`(穩定 id)+ `transcript/speakers.json`(id→display)。

## 4. 錯誤處理
- `merge_ids` 空 / `keep_id` 不存在 → no-op 或明確 Err,不毀檔。
- `set_segment_speaker` 找不到該 seg_id → no-op(回 Ok 或 Err,前端容忍)。
- 寫入沿用 `write_segments_jsonl`(tmp+rename 原子),不留資料遺失窗。
- 任何修正後讓前端重載,確保畫面跟檔案一致。

## 5. 測試
- 純函式 `relabel_merge` / `relabel_one` / `drop_speakers`:多 id 合併、跨軌、找不到 id、空輸入、speakers 過濾。**核心可測,不需 ML/模型。**
- command 薄包(讀檔→純函式→寫檔)沿用既有原子寫,無新風險。

## 6. 範圍
**v1(本 spec)**:合併講者 + 逐段改講者 + 重跑前警告。
**明確不在 v1**:重跑名字時間重疊轉移(A);**聲紋註冊 + 多人認人(另開大 spec)**;逐段「建新講者」;歷史/undo。
