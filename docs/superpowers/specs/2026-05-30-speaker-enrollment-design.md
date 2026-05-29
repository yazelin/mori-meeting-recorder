# 聲紋註冊 + 多人認人(Speaker Enrollment & Identification)設計 — v1

- 日期:2026-05-30
- 狀態:設計已 co-review(對話 brainstorm 通過),待 user review → plan
- 範圍:recorder 端的聲紋註冊 + 分人時自動認人。**共享聲紋庫**設計成可攜(未來 NAS 散佈 / AgentOS 採用),但 v1 只做 recorder 這一側。

## 1. 目標

分人(diarization)是無監督分群,只給匿名「講者N」、短句/變化大時還會把同一人切多份。**註冊聲紋**給每個人一個穩定參考向量 → 分人後**比對 → 標真名**;好處:自動命名、**跨會議一致**、短句也救得回。對齊 mori-desktop `speaker_id` 的 UX 慣例(~30s、cosine、threshold ~0.7、預設不擋/沒註冊就照舊)。

「認人」是**共享能力**(像 whisper 模型):聲紋庫放 `~/.mori`、可攜、帶模型標記,讓未來「公司 NAS 散佈 + AgentOS 共用」是 contract-compatible 的延伸,不用改 recorder。

## 2. 已拍板決策

| 主題 | 決策 |
|------|------|
| 註冊 UI | recorder **一個「人員/聲紋」分頁**(**不開獨立 app**);錄 ~30s、可**累加樣本**、看已註冊名單、刪 |
| 聲紋庫 | 共享、可攜、**帶 embedding 模型標記**;放 `~/.mori/voiceprints/registry.json` |
| embedding 模型 | **沿用分人同一個** 3D-Speaker(`~/.mori/models/3dspeaker-eres2net-zh.onnx`)→ 註冊與分人向量可比;無 Python |
| 比對時機 | 分人後**逐群(per-cluster)**比對:每群算代表聲紋 → cosine ≥ threshold(~0.7)→ 標該人名;否則留「講者N/未知」 |
| 啟用 | **opt-in**:沒註冊任何人 → 完全照舊(匿名分人)。模型缺 → 註冊/比對停用,不報錯 |
| 跨機器硬前提 | 聲紋是「某模型算的向量」,**跨機器共用必須同一 embedding 模型**;registry 記 `embedding_model`,比對只比同模型,不符 → 視 registry 不可用 |

## 3. 架構

### 3.1 聲紋庫格式(`~/.mori/voiceprints/registry.json`,原子寫)
```json
{
  "contract_version": 1,
  "embedding_model": "3dspeaker-eres2net-zh",
  "people": [
    { "id": "p1", "name": "亞澤", "sample_count": 3, "embedding": [/* f32 mean 向量 */] }
  ]
}
```
- `embedding` = 該人多個樣本的**平均向量**(參考聲紋)。`sample_count` 供顯示/重算。
- 可攜:複製這個檔(+ 之後若存原始樣本)到別台 `~/.mori` 即生效(**前提:同 `embedding_model`**)。
- 向量存 JSON inline(幾百個 float,~KB/人)夠用。
- `contract_version` 前向相容(serde default + 版本上限,比照 whisper-server descriptor 作法)。

### 3.2 元件
- `voiceprint.rs`(新):
  - `compute_embedding(wav: &Path) -> Result<Vec<f32>, String>` —— 用 sherpa-onnx `SpeakerEmbeddingExtractor`(分人那顆 embedding 模型)從 WAV 算聲紋。
  - **純函式(可單測)**:`cosine(a, b) -> f32`;`best_match(emb, &[Person], threshold) -> Option<&Person>`;`accumulate_mean(old_mean, old_n, new_emb) -> new_mean`(累加重算平均);registry 讀寫(serde)+ 版本/模型標記判可用。
- commands(`main.rs`):`enroll_voice(name, wav_path?)`(錄好的 temp WAV → compute_embedding → 累加 → 寫 registry)、`list_voiceprints()`、`remove_voiceprint(id)`、`rename_voiceprint(id, name)`、`voiceprint_models_present()`。
- **分人整合**(`postprocess.rs` diarize_session_inner):assign_speakers 產生群後,對每群算代表聲紋(取該群最長/最乾淨的段音檔 embed)→ `best_match` → 命中就把該講者的 `speakers.json` display 設成人名(+ 標記 auto-matched);沒命中留「講者N」。registry 模型不符/缺 → 跳過比對(照舊匿名)。
- **註冊 UI**(recorder 新分頁,沿用 `.mori-*`/custom Select):錄音鈕(沿用既有 mic 擷取錄 ~30s temp WAV)→ 填名 →「註冊 / 補錄」→ `enroll_voice`;名單列出每人 + sample_count + 刪/改名。

### 3.3 跨機器 / AgentOS 接口(設計留好,v1 不做)
registry = 固定路徑的可攜檔 + 文件化格式 + 模型標記 → NAS 散佈(複製檔)、AgentOS 讀/寫同一份 = contract-compatible 延伸,**不改 recorder**(比照 whisper-server「ownership 可遷移」)。未來可比照 `whisper-server-contract.md` 寫一份跨 repo 的「voiceprint registry 契約」跟 AgentOS co-sign。

## 4. 錯誤處理(graceful)
- embedding 模型缺 → 註冊鈕停用 + 提示去 Deps 下載;分人比對跳過(匿名照舊)。
- registry 缺/壞/模型不符 → 視為「無註冊」,分人匿名,不報錯不毀檔。
- enroll 失敗(WAV 壞/embed 失敗)→ Err,不動 registry。
- 寫 registry 用 tmp+rename 原子寫。

## 5. 測試
- 純函式:`cosine`(已知向量)、`best_match`(最近 + threshold 邊界 + 空名單→None)、`accumulate_mean`(平均正確)、registry round-trip + 模型標記不符→不可用 + 版本上限。
- 整合(`#[ignore]`,需模型):`compute_embedding` 對真 WAV;enroll 一段 → 對同人另一段 `best_match` 命中、對別人不命中。

## 6. 範圍
**v1(本 spec)**:recorder 註冊分頁 + 可攜聲紋庫(含模型標記)+ 分人逐群比對自動命名 + 累加樣本。
**不在 v1**:NAS 同步 UI(手動複製即可)、AgentOS 側註冊、跨 repo 契約文件的正式 co-sign(可後補)、會議中即時「這是新的人,要不要現在註冊」、自動挑樣本。
