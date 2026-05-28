# Live Captions + VAD Streaming Transcribe — Design

**Date**: 2026-05-29
**Status**: Spec draft pending user review
**Phase**: BI-5 Phase 2(母 spec `mori-desktop/docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md` 標的 "Live captions (chunk / VAD / streaming)")
**Repo**: mori-meeting-recorder

## 0. 核心洞察 — 邊錄邊轉,不是錄完再轉

Phase 1 是「錄音存雙 WAV → stop 後 batch 轉錄整檔 → export」。yazelin 2026-05-29 點出:**如果做即時轉錄,VAD 天然包含在裡面,而且會議結束時稿子已經完成 — 不需要 stop 後再等轉錄。**

這把原本想成兩個分開的 feature(Live captions + VAD 靜音剪裁)統一成**一條 streaming 軌**:

- 即時字幕 = 即時轉錄的副產品(轉出來的 segment 直接顯示)
- VAD 省轉錄 = 靜音 chunk 根本不送 whisper
- 結束即得稿 = segments 邊錄邊累積 + 邊轉邊寫檔,stop 時稿已完整

| | Phase 1(現在) | Phase 2(本 spec) |
|---|---|---|
| 錄音中 | 只存雙 WAV | 存 WAV **+ 每段即時轉錄寫檔** |
| 靜音處理 | 全錄,stop 後整檔轉 | **靜音 chunk 不送 whisper**(VAD 天然) |
| Stop 後 | batch 轉整檔,等 N 秒 | jsonl 早已 append 完整 → **只彙整 md + timeline(毫秒級)** |
| 拿到稿 | 等轉錄 | **幾乎即時** |
| 當機 | 還沒轉 → 只剩 WAV | 已轉的句子都在 jsonl 裡 → 不丟稿 |

## 1. 整體架構

每軌(SYS / MIC)的 audio thread 從「只寫 WAV」升級成三路並行:

```
audio thread (每軌一條,50ms chunk)
  ├─→ 寫 WAV(完整音檔,原樣不變)
  ├─→ 餵 SignalMeter(VU meter,原樣不變)
  └─→ 餵 VadChunker  ← 新
         │  累積 speech samples,偵測靜音切點(連續 ~600ms 靜音 = 一句講完)
         ▼
      切出一個 speech 段(帶絕對 sample offset)→ 丟給 Transcribe Worker(背景 thread + 佇列)
         │  跑 whisper-cli(短段,通常 < realtime)
         ▼
      出 segments(start_ms / end_ms = chunk 絕對 offset + 段內相對時間)
         ├─→ emit "live-segment" event → Live tab 即時顯示(僅顯示用,漏了不影響檔案)
         └─→ append 寫 transcript/<track>.segments.jsonl  ← 立刻落地,single source of truth
```

**single source of truth = jsonl**:不另存 in-memory 累積。Live tab 顯示走 emit(漏一兩段只是少顯示一行,jsonl 仍完整);export / Sessions 卡 / 最終稿一律從 jsonl 讀回。emit 跟 jsonl 解耦,呼應 [[mori-tauri-emit-listen-race]] 的「事件不可靠 → 不能當唯一資料來源」。

**Stop 流程**:
1. 設 stop_flag,停 capture thread,join WAV writer(finalize)
2. flush VadChunker 最後一段未切的 speech → 丟 worker
3. 等 transcribe worker drain 佇列(因 VAD 去靜音,殘量通常 1-2 秒內)
4. **讀回兩條 jsonl**(已 append 完整)→ 彙整 `meeting.public.md` / `meeting.internal.md` / `timeline.json`(復用 Phase 1 的 `exporter::export`,只是 segments 來源從「batch 結果」換成「讀 jsonl」)
5. **不再 batch 轉整檔**

**失敗降級底線**:無論即時轉錄成不成功,**雙軌 WAV 永遠完整落地**。即時轉錄是「加速拿稿」,不是「唯一拿稿途徑」。whisper deps 沒裝 → 即時轉錄 skip,Live tab 顯示提示,WAV 仍存。

## 2. VAD chunker(`audio/vad.rs`)

### 切點規則(每軌獨立判斷,參數來自 config 不是 const)

| 參數(config 欄位) | 預設 | 作用 |
|---|---|---|
| `silence_split_ms` | 600 | 連續這麼久的靜音 = 一句講完 = 切點。字間微停頓(50-100ms)不夠長,不誤切 → 不斷詞 |
| `silence_threshold_db` | -45 | 低於此 RMS 視為靜音。VU meter 用 -40 判「有沒有人聲」,VAD 用 -45 略寬(避免吃掉句尾尾音) |
| `min_speech_secs` | 0.5 | 比這短的(咳嗽 / 鍵盤 / 短雜音)不送 whisper → 天然去噪 |
| `max_segment_secs` | 20 | 連講不停時的安全上限,到 20 秒強制切一次(這個切點可能落詞中間,但要連講 20 秒不喘才會遇到) |

### VadChunker 介面(純邏輯,無 IO,可單元測試)

```rust
pub struct SpeechSegment {
    pub samples: Vec<i16>,
    pub start_offset_ms: u64,  // 相對該軌 audio thread 第一個 sample 的絕對時間
}

pub struct VadChunker {
    cfg: VadConfig,            // 從 RecorderConfig 來
    speech_buf: Vec<i16>,
    speech_start_offset: u64,  // samples
    total_samples_seen: u64,   // samples,累計
    silence_run_samples: u64,  // 當前連續靜音 sample 數
    in_speech: bool,
}

impl VadChunker {
    pub fn new(cfg: VadConfig) -> Self;

    /// 吃一個 chunk(已算好 rms_db),回傳這次切出的完整 speech 段(可能 0 或 1 段)。
    /// chunk_rms_db: 該 chunk 的 RMS(linux.rs/windows.rs 已用 compute_levels 算)
    pub fn push(&mut self, samples: &[i16], chunk_rms_db: f32) -> Option<SpeechSegment>;

    /// stop 時呼叫,把 buffer 內剩餘 speech(若 >= min_speech)吐出來。
    pub fn flush(&mut self) -> Option<SpeechSegment>;
}
```

切點邏輯(`push`):
- chunk RMS >= threshold(有聲)→ silence_run 歸零;若原本 not in_speech,標 in_speech、記 speech_start_offset
- chunk RMS < threshold(靜音)→ silence_run += chunk samples;若 in_speech 且 silence_run >= silence_split → 切:speech_buf 若 >= min_speech 則吐 SpeechSegment,清 buf,標 not in_speech
- in_speech 時無論有聲靜音都把 samples push 進 speech_buf(尾段靜音也含進去,whisper 較準)
- speech_buf 長度 >= max_segment → 強制吐(in_speech 維持 true,接著的 samples 開新段)
- total_samples_seen 每次 += samples.len()(算 offset 用)

### 時間軸對齊(yazelin 擔心的 desync)

每段 `start_offset_ms = speech_start_offset_samples / 16`(16kHz mono)。whisper 跑該段出來的 segment 時間是段內相對,最終 `segment.start_ms = start_offset_ms + 段內相對 ms`。

SYS / MIC 各自切各自,但都用「該軌第一個 sample」當 offset 原點,兩軌同時 start_session → **絕對時間軸對齊**。

## 3. Transcribe worker(`transcribe.rs` 改)

每軌一個背景 thread + `std::sync::mpsc` 佇列:

```rust
pub struct TranscribeWorker {
    tx: Sender<SpeechSegment>,   // VadChunker 切出的段丟進來
    handle: JoinHandle<()>,
    pending: Arc<AtomicUsize>,   // 佇列待處理數,給 Live tab「轉錄中 N 段」顯示
}
```

worker loop:
1. recv 一個 SpeechSegment
2. 寫 temp WAV(`/tmp/mori-live-<uuid>.wav`)
3. `run_whisper(temp_wav, session_id, kind)` → Vec<Segment>(段內相對時間)
4. 每個 segment 的 start_ms / end_ms 加上 `seg.start_offset_ms`(轉絕對時間)
5. **append 寫 `transcript/<track>.segments.jsonl`**(一行一 segment,跟 Phase 1 batch 格式一致)
6. emit `"live-segment"` event(payload: track + segment)
7. unlink temp WAV;pending -= 1

**跟不上不丟資料**:佇列積壓只是 Live 字幕延遲;WAV 完整存著;stop 時等 drain。
**deps 缺**:run_whisper 回空 vec,worker skip,WAV 仍存。

## 4. Config(`config.rs` 新)

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct RecorderConfig {
    pub silence_split_ms: u64,        // 600
    pub silence_threshold_db: f32,    // -45.0
    pub min_speech_secs: f32,         // 0.5
    pub max_segment_secs: f32,        // 20.0
}

impl Default for RecorderConfig { /* 上述預設 */ }

pub fn config_path() -> PathBuf;          // ~/.mori/meeting-recorder/config.json
pub fn read_config() -> RecorderConfig;   // 缺檔/parse fail/缺欄 → 各自回預設
pub fn write_config(cfg: &RecorderConfig) -> Result<(), String>;
```

缺欄回預設:用 serde `#[serde(default = "...")]` per-field,舊 config 加新欄不會壞。

`VadConfig` 是 RecorderConfig 的 VAD 子集(目前等同),VadChunker 吃它。

## 5. Tauri commands + events(`main.rs` 改)

| Command / Event | 方向 | 內容 |
|---|---|---|
| `get_config` | command | 回 `RecorderConfig` |
| `set_config(config)` | command | 存檔;回 Result |
| `"live-segment"` | event(Rust→JS) | `{ track: "sys"\|"mic", segment: Segment }` |

`recorder_start` 開始時 `read_config()` 拿參數建 VadChunker(改參數下次錄音生效,不熱套用)。

## 6. 前端

```
src/tabs/LiveTab.tsx           (新)— listen "live-segment",雙欄滾動
src/tabs/SettingsTab.tsx       (新)— get_config/set_config,參數表單 + 還原預設
src/components/LiveColumn.tsx  (新)— 單欄(SYS or MIC)字幕滾動 + auto-scroll
src/components/SettingField.tsx (新)— 一個參數列(label / input / unit / 預設提示 / 一行說明)
src/ExpandedView.tsx           (改)— tab bar 加 Live + Settings(共 5 tab)
src/i18n/locales/{en,zh-TW}.json (改)— live.* / settings.* keys
```

### Live tab 版面

雙欄:左 SYS(對外)/ 右 MIC(內部),延續「客戶版只有 SYS」的視覺分離。每行 `時間戳 + 文字`,新 segment 從底長出 + auto-scroll。沒錄音 → placeholder。worker 積壓 → 底部「▏轉錄中…(N 段待處理)」。

LiveTab 維護兩個 segment array(sys / mic),listen "live-segment" push 對應 array。切 tab 離開不清空(回來還在),stop 後保留到下次 start 才清。

### Settings tab 版面

4 個 SettingField:每個 `標籤 + 數字 input + 單位 + 預設值提示 + 一行作用說明`。底部「還原預設」+「儲存」。儲存呼 set_config。改了提示「下次錄音生效」。

### 5-tab 寬度

720px expanded 寬,5 個 tab(Record/Live/Sessions/Deps/Settings)+ collapse/close,每 tab pill ~70px 放得下。tab bar 已是 flex,不需特別處理。

## 7. 測試策略

### Unit(cargo test)

| 模組 | cases |
|---|---|
| `config.rs` | round-trip 讀寫;缺檔回預設;部分欄位缺各自回預設(serde default) |
| `audio/vad.rs` | **重點**:(a) 連續 speech + 600ms 靜音 → 切一段,start_offset 正確;(b) 字間 100ms 微停頓不切;(c) < min_speech 段不吐;(d) max_segment 強制切;(e) 連續多段 offset 累加正確;(f) flush 吐剩餘段 |
| `transcribe.rs` | segment 絕對時間 = offset + 段內相對(用 fixture json);append jsonl 格式跟 Phase 1 一致 |

### 手動 e2e

- 播 YouTube + 對麥講話 → Live tab 雙欄即時出字幕,SYS 左 MIC 右
- 講一句停一下 → 字幕一句句出現(VAD 切點)
- 靜音時段 → 不出空 segment(VAD 去靜音)
- Stop → 幾乎立刻完成(不等 batch);Sessions 卡 segs 數 > 0;public.md 不空
- Settings 改 silence_split_ms → 存 → 下次錄音切點變化
- 中途 kill process → jsonl 已有先前句子(不丟稿)

### 通用 gate

`bash scripts/verify.sh` 全綠(新 vad/config 測試 + 既有 + npm build + cargo check)。

## 8. 風險與限制

| 風險 | 影響 | 緩解 |
|---|---|---|
| whisper 比 realtime 慢 → 佇列積壓 | Live 字幕延遲;stop 要等 drain | VAD 去靜音大幅減量;pending 計數顯示;WAV 完整不丟 |
| max_segment 切點落詞中間 | 該處斷詞 | 要連講 20 秒不停才遇到,罕見;可調 |
| 短 speech 段 whisper 準度低 | 短句字幕可能不準 | min_speech 過濾極短;短段本來資訊量少 |
| Stop 流程大改(不再 batch) | 回歸風險 | plan 切獨立 task,保留 WAV 當底;e2e 驗 segs > 0 |
| Windows VAD/worker 未親測 | Windows 表現未知 | spec 標「未驗 Windows」;Linux 先行 |
| 5 tab 在 720px 可能略擠 | 視覺 | tab pill 已 flex;真擠再縮字 |

## 9. 刻意不做(Phase 邊界)

- LLM 重整主旨稿(基於會議主題 / 參與人員 / 主旨稿)→ Phase 3
- Speaker diarization(分辨誰在講)→ Phase 4
- 錄音中熱套用參數(改參數下次錄音才生效)
- whisper 中途 deps 才裝好的「補轉」(WAV 完整,可日後加)
- mori-ear 靜音剪裁(不同 repo / 不同軌,另議)
- overlap window 去重(yazelin 明確不要邊緣瑕疵,改用 VAD 靜音切點)

## 10. 下一步

spec 過 → `superpowers:writing-plans` 拆 plan → subagent-driven 落地。Branch 命名 `feat/*`(對齊 [[mori-branch-naming]]),trunk-based + 短命 branch([[feedback_trunk_based_auto_merge]])。動 Rust audio loop 需 restart-dev 才吃新 binary。

---

**Related memories**: [[reference_tauri2_gotchas]](spawn / capability)、[[mori-tauri-emit-listen-race]](emit + polling fallback)、[[feedback_trunk_based_auto_merge]]、[[mori-branch-naming]]
**母 spec**: `mori-desktop/docs/superpowers/specs/2026-05-28-bi-5-meeting-recorder-design.md`
**Phase 1 spec**: `docs/superpowers/specs/2026-05-28-recorder-ui-mock-alignment-design.md`
