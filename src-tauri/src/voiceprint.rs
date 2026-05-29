//! 聲紋註冊 + 認人。registry 在 ~/.mori/voiceprints/registry.json(可攜、帶 embedding 模型標記)。
//! 嵌入用分人那顆 sherpa-onnx 3D-Speaker(無 Python);比對用自家 cosine(純、可測)。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SUPPORTED_VOICEPRINT_VERSION: u32 = 1;
/// 目前 embedding 模型識別(= ~/.mori/models/3dspeaker-eres2net-zh.onnx)。跨機器共用聲紋必須同此值。
pub const EMB_MODEL: &str = "3dspeaker-eres2net-zh";
pub const MATCH_THRESHOLD: f32 = 0.7;
fn default_version() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person { pub id: String, pub name: String, pub samples: Vec<Vec<f32>> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default = "default_version")] pub contract_version: u32,
    pub embedding_model: String,
    pub people: Vec<Person>,
}

pub fn registry_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mori").join("voiceprints").join("registry.json")
}

/// cosine 相似度(同長度;零向量回 0)。
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() { return 0.0; }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

/// query 對每個人取其 samples 的最大 cosine;最佳 ≥ threshold → Some(該人)。空/都不夠像 → None。
pub fn best_match<'a>(query: &[f32], people: &'a [Person], threshold: f32) -> Option<&'a Person> {
    people.iter()
        .map(|p| (p, p.samples.iter().map(|e| cosine(query, e)).fold(f32::MIN, f32::max)))
        .filter(|(_, s)| *s >= threshold)
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(p, _)| p)
}

/// 讀 registry;缺/壞/版本太新/模型不符 → None(視為無註冊)。
pub fn read_registry() -> Option<Registry> {
    let s = std::fs::read_to_string(registry_path()).ok()?;
    let reg: Registry = serde_json::from_str(&s).ok()?;
    if reg.contract_version > SUPPORTED_VOICEPRINT_VERSION { return None; }
    if reg.embedding_model != EMB_MODEL {
        eprintln!("[voiceprint] registry model {} != {EMB_MODEL} — ignoring", reg.embedding_model);
        return None;
    }
    Some(reg)
}

/// 讀回供「寫入」用(不做模型 gate,讓 enroll 能初始化/沿用)。缺 → 新的空 registry(本機模型)。
pub fn load_or_new() -> Registry {
    std::fs::read_to_string(registry_path()).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| Registry { contract_version: 1, embedding_model: EMB_MODEL.to_string(), people: vec![] })
}

pub fn write_registry(reg: &Registry) -> Result<(), String> {
    let path = registry_path();
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| format!("mkdir: {e}"))?; }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(reg).map_err(|e| e.to_string())?).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))
}

/// 從 f32 樣本算聲紋向量(sherpa-onnx 3D-Speaker,= 分人那顆 emb 模型)。模型缺 → Err。
/// sample_rate 用 i32 對齊 sherpa-onnx accept_waveform 簽名。
pub fn embed_samples(samples: &[f32], sample_rate: i32) -> Result<Vec<f32>, String> {
    use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};
    let model = crate::diarize::emb_model_path();
    if !model.exists() { return Err("embedding model not installed".into()); }
    let config = SpeakerEmbeddingExtractorConfig {
        model: Some(model.to_string_lossy().to_string()),
        num_threads: 1, debug: false, provider: Some("cpu".to_string()),
    };
    let extractor = SpeakerEmbeddingExtractor::create(&config).ok_or("create extractor")?;
    let stream = extractor.create_stream().ok_or("create stream")?;
    stream.accept_waveform(sample_rate, samples);
    stream.input_finished();
    extractor.compute(&stream).ok_or_else(|| "compute embedding failed".to_string())
}

/// 讀整個 WAV 算聲紋(註冊用;16k mono)。
pub fn embed_wav_file(path: &std::path::Path) -> Result<Vec<f32>, String> {
    use sherpa_onnx::Wave;
    let wave = Wave::read(&path.to_string_lossy()).ok_or_else(|| format!("read wave {}", path.display()))?;
    embed_samples(wave.samples(), wave.sample_rate())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn person(id: &str, name: &str, samples: Vec<Vec<f32>>) -> Person { Person { id: id.into(), name: name.into(), samples } }

    #[test]
    fn cosine_basic() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0); // 長度不符
    }

    #[test]
    fn best_match_picks_nearest_above_threshold() {
        let people = vec![
            person("p1", "亞澤", vec![vec![1.0, 0.0]]),
            person("p2", "老闆", vec![vec![0.0, 1.0]]),
        ];
        assert_eq!(best_match(&[0.95, 0.05], &people, 0.7).map(|p| p.name.as_str()), Some("亞澤"));
        assert!(best_match(&[0.7, 0.7], &people, 0.99).is_none()); // 都不夠像
        assert!(best_match(&[1.0, 0.0], &[], 0.7).is_none());      // 空名單
    }

    #[test]
    fn registry_round_trip_and_gates() {
        // (用 load_or_new/parse 純邏輯;registry_path 真檔 IO 不在純測)
        let reg = Registry { contract_version: 1, embedding_model: EMB_MODEL.into(), people: vec![person("p1","甲",vec![vec![0.1,0.2]])] };
        let s = serde_json::to_string(&reg).unwrap();
        let back: Registry = serde_json::from_str(&s).unwrap();
        assert_eq!(back.people[0].name, "甲");
        // 版本太新 / 模型不符 由 read_registry gate(見實作);此處驗 parse 不爆
        let newer = r#"{"contract_version":2,"embedding_model":"x","people":[]}"#;
        let _: Registry = serde_json::from_str(newer).unwrap();
    }

    #[test]
    #[ignore]
    fn embed_wav_real() {
        // EMB_WAV=/path/to.wav cargo test --release embed_wav_real -- --ignored --nocapture
        let p = std::env::var("EMB_WAV").expect("set EMB_WAV");
        let e = embed_wav_file(std::path::Path::new(&p)).expect("embed");
        assert!(!e.is_empty());
        eprintln!("dim={}", e.len());
    }
}
