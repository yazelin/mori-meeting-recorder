//! 講者分離的 crate 無關核心:型別 + assign_speakers 對齊(純函式,可單測)。
//! 引擎(diarize_wav,依賴選定的 onnx crate)由 Plan B 於本檔加入。

use crate::transcribe::Segment;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── 模型路徑 helpers ─────────────────────────────────────────────────────────

/// segmentation 模型路徑(下載時 rename 成這個固定名)。
pub fn seg_model_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mori").join("models").join("pyannote-segmentation-3-0.onnx")
}

/// speaker embedding 模型路徑。
pub fn emb_model_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mori").join("models").join("3dspeaker-eres2net-zh.onnx")
}

/// 兩個模型都在才算裝好。
pub fn diarization_models_present() -> bool {
    seg_model_path().exists() && emb_model_path().exists()
}

// ── diarize_wav 引擎(sherpa-onnx) ───────────────────────────────────────────

/// 對單一 WAV 跑 sherpa-onnx 講者分離。`num_clusters`:Some(n>0) 用已知人數(品質最佳,
/// 來自 meeting-info 人員數);None → 自動(cluster threshold,易過/欠切,使用者改名時修)。
/// 回 SpeakerSpan(該軌 local speaker id)。模型缺 / 引擎錯 → Err(caller 視為「該軌不標」)。
pub fn diarize_wav(wav: &std::path::Path, num_clusters: Option<usize>) -> Result<Vec<SpeakerSpan>, String> {
    use sherpa_onnx::{
        FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
        OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
        SpeakerEmbeddingExtractorConfig, Wave,
    };
    let seg = seg_model_path();
    let emb = emb_model_path();
    if !seg.exists() || !emb.exists() {
        return Err("diarization models not installed".to_string());
    }
    // num_clusters>0 用已知人數;否則 -1 = 交給 threshold(起點 0.7,spike 觀察 0.5 過切)。
    let clustering = match num_clusters {
        Some(n) if n > 0 => FastClusteringConfig { num_clusters: n as i32, ..Default::default() },
        _ => FastClusteringConfig { num_clusters: -1, threshold: 0.7 },
    };
    let config = OfflineSpeakerDiarizationConfig {
        segmentation: OfflineSpeakerSegmentationModelConfig {
            pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                model: Some(seg.to_string_lossy().to_string()),
            },
            ..Default::default()
        },
        embedding: SpeakerEmbeddingExtractorConfig {
            model: Some(emb.to_string_lossy().to_string()),
            ..Default::default()
        },
        clustering,
        ..Default::default()
    };
    let sd = OfflineSpeakerDiarization::create(&config)
        .ok_or_else(|| "create diarizer failed (check model paths / onnxruntime)".to_string())?;
    let wave = Wave::read(&wav.to_string_lossy())
        .ok_or_else(|| format!("read wave {} failed (check file exists + mono WAV)", wav.display()))?;
    if sd.sample_rate() != wave.sample_rate() {
        return Err(format!("sample-rate mismatch: model {} vs wav {}", sd.sample_rate(), wave.sample_rate()));
    }
    let result = sd.process(wave.samples())
        .ok_or_else(|| "diarize process returned None".to_string())?;
    let spans = result
        .sort_by_start_time()
        .into_iter()
        .map(|s| SpeakerSpan {
            start_ms: (s.start.max(0.0) * 1000.0) as u64,
            end_ms: (s.end.max(0.0) * 1000.0) as u64,
            speaker_local: s.speaker.max(0) as usize,
        })
        .collect();
    Ok(spans)
}

// ── participant_count ─────────────────────────────────────────────────────────

/// 從 meeting-info 的人員字串數人數(逗號 , 、頓號 、 、分號 ; 、換行 皆分隔);空 → None。
pub fn participant_count(participants: &str) -> Option<usize> {
    let n = participants
        .split(|c| c == ',' || c == '、' || c == ';' || c == '\n' || c == '，')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .count();
    if n > 0 { Some(n) } else { None }
}

/// 一個講者-同質時間段(引擎輸出);speaker_local = 該軌內的本地群 id(0-based)。
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerSpan {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_local: usize,
}

/// 統一後的講者(跨軌 S1..Sn);display 預設「講者N」,track = 來源軌。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerInfo {
    pub id: String,
    pub display: String,
    pub track: String,
}

/// 一軌的 diarization 輸入:該軌名、spans、該軌 segments。
pub struct TrackDiarization {
    pub track: String,
    pub spans: Vec<SpeakerSpan>,
    pub segments: Vec<Segment>,
}

// mixed 門檻:次多講者重疊 >= min(1s, 30% 段長) → 標 speaker_mixed(取小者,短段更易被標)。
const MIXED_MIN_MS: u64 = 1000;
const MIXED_MIN_FRAC: f64 = 0.30;

/// 把每軌 spans 對齊到該軌 segments:多數重疊賦 speaker(統一 S1..Sn),次多顯著 → speaker_mixed。
/// 跨軌統一編號(一人只在一軌)。回 (標好的 segments, 講者表)。
pub fn assign_speakers(tracks: Vec<TrackDiarization>) -> (Vec<Segment>, Vec<SpeakerInfo>) {
    let mut speakers: Vec<SpeakerInfo> = Vec::new();
    let mut out: Vec<Segment> = Vec::new();
    let mut next_global = 1usize;

    for td in tracks {
        // 本地 id → 全域 S{n}(本地 id 排序,決定性)
        let mut local_ids: Vec<usize> = td.spans.iter().map(|s| s.speaker_local).collect();
        local_ids.sort_unstable();
        local_ids.dedup();
        let mut map: HashMap<usize, String> = HashMap::new();
        for lid in local_ids {
            let gid = format!("S{next_global}");
            speakers.push(SpeakerInfo {
                id: gid.clone(),
                display: format!("講者{next_global}"),
                track: td.track.clone(),
            });
            map.insert(lid, gid);
            next_global += 1;
        }

        for mut s in td.segments {
            let mut overlap: HashMap<usize, u64> = HashMap::new();
            for span in &td.spans {
                let lo = s.start_ms.max(span.start_ms);
                let hi = s.end_ms.min(span.end_ms);
                if hi > lo {
                    *overlap.entry(span.speaker_local).or_insert(0) += hi - lo;
                }
            }
            if overlap.is_empty() {
                s.speaker = None;
                s.speaker_mixed = false;
            } else {
                let mut v: Vec<(usize, u64)> = overlap.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                s.speaker = map.get(&v[0].0).cloned();
                let dur = s.end_ms.saturating_sub(s.start_ms).max(1);
                let threshold = ((dur as f64 * MIXED_MIN_FRAC) as u64).min(MIXED_MIN_MS);
                let second = v.get(1).map(|x| x.1).unwrap_or(0);
                s.speaker_mixed = second > 0 && second >= threshold;
            }
            out.push(s);
        }
    }
    (out, speakers)
}

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
struct SpeakerEntry {
    display: String,
    track: String,
}

/// 寫 speakers.json:物件 id→{display,track}(BTreeMap 穩定排序),原子寫(tmp+rename)。
pub fn write_speakers(path: &Path, speakers: &[SpeakerInfo]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let map: BTreeMap<String, SpeakerEntry> = speakers
        .iter()
        .map(|s| (s.id.clone(), SpeakerEntry { display: s.display.clone(), track: s.track.clone() }))
        .collect();
    let body = serde_json::to_string_pretty(&map).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))
}

/// 讀 speakers.json → Vec<SpeakerInfo>(依 id 排序)。缺檔/壞檔 → 空。
pub fn read_speakers(path: &Path) -> Vec<SpeakerInfo> {
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let map: BTreeMap<String, SpeakerEntry> = match serde_json::from_str(&s) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    map.into_iter()
        .map(|(id, e)| SpeakerInfo { id, display: e.display, track: e.track })
        .collect()
}

/// 改某講者的顯示名(只動 speakers.json)。找不到 id → Err。
pub fn rename_speaker(path: &Path, id: &str, new_display: &str) -> Result<(), String> {
    let mut speakers = read_speakers(path);
    let found = speakers.iter_mut().find(|s| s.id == id);
    match found {
        Some(s) => {
            s.display = new_display.to_string();
            write_speakers(path, &speakers)
        }
        None => Err(format!("speaker id not found: {id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 模型路徑 + participant_count 純函式測試 ──────────────────────────────

    #[test]
    fn diar_model_paths_under_mori_models() {
        assert!(seg_model_path().to_string_lossy().contains(".mori/models") || seg_model_path().to_string_lossy().contains(".mori\\models"));
        assert!(emb_model_path().ends_with("3dspeaker-eres2net-zh.onnx"));
    }

    #[test]
    fn participant_count_counts_names() {
        assert_eq!(participant_count("亞澤, 老闆、阿明\n小美"), Some(4));
        assert_eq!(participant_count("  "), None);
        assert_eq!(participant_count(""), None);
        assert_eq!(participant_count("只有我"), Some(1));
    }

    /// 需 ~/.mori/models 的兩個 diar 模型 + 一個多人 wav。手動:
    ///   DIAR_WAV=/path/0-four-speakers-zh.wav cargo test --release diarize_wav_real -- --ignored --nocapture
    #[test]
    #[ignore]
    fn diarize_wav_real() {
        let wav = std::env::var("DIAR_WAV").expect("set DIAR_WAV");
        let spans = diarize_wav(std::path::Path::new(&wav), Some(4)).expect("diarize");
        eprintln!("got {} spans", spans.len());
        assert!(!spans.is_empty());
        let speakers: std::collections::BTreeSet<usize> = spans.iter().map(|s| s.speaker_local).collect();
        assert!(speakers.len() >= 2, "expected ≥2 speakers, got {}", speakers.len());
    }

    // ── assign_speakers / speakers.json 既有測試 ────────────────────────────

    fn seg(track: &str, source: &str, vis: &str, start: u64, end: u64) -> Segment {
        Segment {
            id: format!("{track}-{start}"),
            session_id: "m".into(),
            track: track.into(),
            source_kind: source.into(),
            visibility: vis.into(),
            start_ms: start,
            end_ms: end,
            text: "x".into(),
            is_final: true,
            confidence: None,
            speaker: None,
            speaker_mixed: false,
        }
    }

    #[test]
    fn single_speaker_segment_gets_label_not_mixed() {
        let td = TrackDiarization {
            track: "system".into(),
            spans: vec![SpeakerSpan { start_ms: 0, end_ms: 5000, speaker_local: 0 }],
            segments: vec![seg("system", "meeting_system", "public", 1000, 2000)],
        };
        let (segs, speakers) = assign_speakers(vec![td]);
        assert_eq!(segs[0].speaker.as_deref(), Some("S1"));
        assert!(!segs[0].speaker_mixed);
        assert_eq!(speakers, vec![SpeakerInfo { id: "S1".into(), display: "講者1".into(), track: "system".into() }]);
    }

    #[test]
    fn two_speaker_segment_is_mixed_and_takes_majority() {
        // 段 0..2000:講者0 佔 0..1600(多數)、講者1 佔 1600..2000(400ms);
        // 段長 2000 → 門檻 min(600, 1000)=600;次多 400 < 600 → 不 mixed
        let td = TrackDiarization {
            track: "system".into(),
            spans: vec![
                SpeakerSpan { start_ms: 0, end_ms: 1600, speaker_local: 0 },
                SpeakerSpan { start_ms: 1600, end_ms: 5000, speaker_local: 1 },
            ],
            segments: vec![seg("system", "meeting_system", "public", 0, 2000)],
        };
        let (segs, _) = assign_speakers(vec![td]);
        assert_eq!(segs[0].speaker.as_deref(), Some("S1")); // 多數 = local 0 = S1
        assert!(!segs[0].speaker_mixed);

        // 段 0..2000:講者0 佔 0..1000、講者1 佔 1000..2000(1000ms);門檻 min(600,1000)=600;次多 1000>=600 → mixed
        let td2 = TrackDiarization {
            track: "system".into(),
            spans: vec![
                SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 },
                SpeakerSpan { start_ms: 1000, end_ms: 5000, speaker_local: 1 },
            ],
            segments: vec![seg("system", "meeting_system", "public", 0, 2000)],
        };
        let (segs2, _) = assign_speakers(vec![td2]);
        assert!(segs2[0].speaker_mixed);
    }

    #[test]
    fn no_overlap_segment_has_no_speaker() {
        let td = TrackDiarization {
            track: "system".into(),
            spans: vec![SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 }],
            segments: vec![seg("system", "meeting_system", "public", 5000, 6000)],
        };
        let (segs, _) = assign_speakers(vec![td]);
        assert_eq!(segs[0].speaker, None);
        assert!(!segs[0].speaker_mixed);
    }

    #[test]
    fn cross_track_numbering_is_unified_and_continues() {
        let sys = TrackDiarization {
            track: "system".into(),
            spans: vec![
                SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 },
                SpeakerSpan { start_ms: 1000, end_ms: 2000, speaker_local: 1 },
            ],
            segments: vec![seg("system", "meeting_system", "public", 0, 1000)],
        };
        let mic = TrackDiarization {
            track: "mic-internal".into(),
            spans: vec![SpeakerSpan { start_ms: 0, end_ms: 1000, speaker_local: 0 }],
            segments: vec![seg("mic-internal", "mic_internal", "internal", 0, 1000)],
        };
        let (segs, speakers) = assign_speakers(vec![sys, mic]);
        // sys 兩群 → S1,S2;mic 一群 → S3
        assert_eq!(speakers.iter().map(|s| s.id.clone()).collect::<Vec<_>>(), vec!["S1", "S2", "S3"]);
        assert_eq!(speakers[2].track, "mic-internal");
        assert_eq!(segs[0].speaker.as_deref(), Some("S1"));
        assert_eq!(segs[1].speaker.as_deref(), Some("S3"));
    }

    #[test]
    fn speakers_json_round_trip_and_rename() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("speakers.json");
        let speakers = vec![
            SpeakerInfo { id: "S1".into(), display: "講者1".into(), track: "system".into() },
            SpeakerInfo { id: "S2".into(), display: "講者2".into(), track: "mic-internal".into() },
        ];
        write_speakers(&path, &speakers).unwrap();
        let read = read_speakers(&path);
        assert_eq!(read, speakers);

        rename_speaker(&path, "S1", "亞澤").unwrap();
        let read2 = read_speakers(&path);
        assert_eq!(read2.iter().find(|s| s.id == "S1").unwrap().display, "亞澤");
        assert_eq!(read2.iter().find(|s| s.id == "S2").unwrap().display, "講者2");
    }

    #[test]
    fn read_speakers_missing_file_is_empty() {
        assert!(read_speakers(std::path::Path::new("/nonexistent/speakers.json")).is_empty());
    }

    #[test]
    fn empty_spans_yields_no_speaker() {
        let td = TrackDiarization {
            track: "system".into(),
            spans: vec![],
            segments: vec![seg("system", "meeting_system", "public", 0, 1000)],
        };
        let (segs, speakers) = assign_speakers(vec![td]);
        assert_eq!(segs[0].speaker, None);
        assert!(!segs[0].speaker_mixed);
        assert!(speakers.is_empty());
    }
}
