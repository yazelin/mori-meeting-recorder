//! VAD chunker — 偵測靜音切點,把連續 audio 切成 speech 段。純邏輯,無 IO。
//!
//! 喂法:audio loop 每個 50ms chunk 算好 rms_db 後呼 push()。chunk RMS >= threshold
//! 算有聲,< threshold 算靜音。連續靜音超過 silence_split → 切。speech 段累積到
//! max_segment → 強制切。< min_speech 的段丟掉(去噪)。

const SAMPLE_RATE: u64 = 16_000;

#[derive(Debug, Clone)]
pub struct VadConfig {
    pub silence_split_ms: u64,
    pub silence_threshold_db: f32,
    pub min_speech_secs: f32,
    pub max_segment_secs: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechSegment {
    pub samples: Vec<i16>,
    pub start_offset_ms: u64, // 相對該軌第一個 sample 的絕對時間
}

pub struct VadChunker {
    cfg: VadConfig,
    speech_buf: Vec<i16>,
    speech_start_offset_samples: u64,
    total_samples_seen: u64,
    silence_run_samples: u64,
    in_speech: bool,
}

impl VadChunker {
    pub fn new(cfg: VadConfig) -> Self {
        Self {
            cfg,
            speech_buf: Vec::new(),
            speech_start_offset_samples: 0,
            total_samples_seen: 0,
            silence_run_samples: 0,
            in_speech: false,
        }
    }

    fn silence_split_samples(&self) -> u64 {
        self.cfg.silence_split_ms * SAMPLE_RATE / 1000
    }
    fn min_speech_samples(&self) -> u64 {
        (self.cfg.min_speech_secs * SAMPLE_RATE as f32) as u64
    }
    fn max_segment_samples(&self) -> u64 {
        (self.cfg.max_segment_secs * SAMPLE_RATE as f32) as u64
    }

    /// 吃一個 chunk(samples + 已算好的 rms_db)。回傳這次切出的完整 speech 段(0 或 1)。
    pub fn push(&mut self, samples: &[i16], rms_db: f32) -> Option<SpeechSegment> {
        let chunk_start = self.total_samples_seen;
        self.total_samples_seen += samples.len() as u64;
        let is_voice = rms_db >= self.cfg.silence_threshold_db;

        if is_voice {
            if !self.in_speech {
                self.in_speech = true;
                self.speech_start_offset_samples = chunk_start;
                self.speech_buf.clear();
            }
            self.silence_run_samples = 0;
            self.speech_buf.extend_from_slice(samples);
        } else if self.in_speech {
            // 靜音但在 speech 中:尾段靜音也含進去(whisper 較準),累計 silence run
            self.speech_buf.extend_from_slice(samples);
            self.silence_run_samples += samples.len() as u64;
            if self.silence_run_samples >= self.silence_split_samples() {
                return self.cut();
            }
        }
        // max_segment 強制切(就算還在連續講)
        if self.in_speech && self.speech_buf.len() as u64 >= self.max_segment_samples() {
            return self.cut_forced();
        }
        None
    }

    /// 一般切(靜音觸發):吐段 if >= min_speech,然後離開 speech 狀態。
    fn cut(&mut self) -> Option<SpeechSegment> {
        let seg = self.take_if_long_enough();
        self.in_speech = false;
        self.silence_run_samples = 0;
        seg
    }

    /// 強制切(max_segment):吐段,但維持 in_speech,新段從目前位置開始。
    fn cut_forced(&mut self) -> Option<SpeechSegment> {
        let seg = self.take_if_long_enough();
        self.speech_start_offset_samples = self.total_samples_seen;
        self.silence_run_samples = 0;
        seg
    }

    fn take_if_long_enough(&mut self) -> Option<SpeechSegment> {
        // min_speech 量的是「語音長度」,要扣掉尾段靜音(speech_buf 含尾段靜音讓
        // whisper 較準,但那段靜音不該算進「這段有沒有夠長的語音」的判斷)。
        let voice_len = (self.speech_buf.len() as u64).saturating_sub(self.silence_run_samples);
        if voice_len >= self.min_speech_samples() {
            let samples = std::mem::take(&mut self.speech_buf);
            Some(SpeechSegment {
                samples,
                start_offset_ms: self.speech_start_offset_samples * 1000 / SAMPLE_RATE,
            })
        } else {
            self.speech_buf.clear();
            None
        }
    }

    /// stop 時呼叫,吐剩餘 speech 段(若 >= min_speech)。
    pub fn flush(&mut self) -> Option<SpeechSegment> {
        if self.in_speech {
            let seg = self.take_if_long_enough();
            self.in_speech = false;
            seg
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> VadConfig {
        VadConfig {
            silence_split_ms: 600,
            silence_threshold_db: -45.0,
            min_speech_secs: 0.5,
            max_segment_secs: 20.0,
        }
    }

    // 50ms chunk @16kHz = 800 samples
    const CHUNK: usize = 800;
    fn voice_chunk() -> Vec<i16> {
        vec![5000; CHUNK]
    }
    fn silent_chunk() -> Vec<i16> {
        vec![0; CHUNK]
    }

    const VOICE_DB: f32 = -20.0; // >= -45 → voice
    const SILENT_DB: f32 = -90.0; // < -45 → silence

    #[test]
    fn speech_then_600ms_silence_cuts_one_segment() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..20 {
            assert!(v.push(&voice_chunk(), VOICE_DB).is_none());
        }
        // 靜音:600ms = 12 chunks。前 11 個不切,第 12 個切。
        for _ in 0..11 {
            assert!(v.push(&silent_chunk(), SILENT_DB).is_none());
        }
        let seg = v.push(&silent_chunk(), SILENT_DB);
        assert!(seg.is_some(), "12th silent chunk should trigger cut");
        let seg = seg.unwrap();
        assert_eq!(seg.start_offset_ms, 0);
        // samples = 20 voice + 12 silence = 32 chunks(含尾段靜音)
        assert_eq!(seg.samples.len(), 32 * CHUNK);
    }

    #[test]
    fn short_inter_word_silence_does_not_cut() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..20 {
            v.push(&voice_chunk(), VOICE_DB);
        }
        // 100ms 靜音 = 2 chunks(< 600ms),不切
        assert!(v.push(&silent_chunk(), SILENT_DB).is_none());
        assert!(v.push(&silent_chunk(), SILENT_DB).is_none());
        // 又有聲,silence_run 應歸零
        assert!(v.push(&voice_chunk(), VOICE_DB).is_none());
    }

    #[test]
    fn too_short_speech_dropped() {
        let mut v = VadChunker::new(cfg());
        // 0.25s speech = 5 chunks(< min_speech 0.5s = 10 chunks)
        for _ in 0..5 {
            v.push(&voice_chunk(), VOICE_DB);
        }
        for _ in 0..11 {
            v.push(&silent_chunk(), SILENT_DB);
        }
        let seg = v.push(&silent_chunk(), SILENT_DB);
        assert!(seg.is_none(), "sub-min_speech segment should be dropped");
    }

    #[test]
    fn max_segment_forces_cut() {
        let mut v = VadChunker::new(cfg());
        // 連續講,max_segment 20s = 320000 samples = 400 chunks。
        let mut cut_seen = false;
        for _ in 0..401 {
            if v.push(&voice_chunk(), VOICE_DB).is_some() {
                cut_seen = true;
            }
        }
        assert!(cut_seen, "max_segment should force a cut within 401 chunks");
    }

    #[test]
    fn consecutive_segments_have_increasing_offset() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..20 {
            v.push(&voice_chunk(), VOICE_DB);
        }
        for _ in 0..11 {
            v.push(&silent_chunk(), SILENT_DB);
        }
        let seg1 = v.push(&silent_chunk(), SILENT_DB).unwrap();
        assert_eq!(seg1.start_offset_ms, 0);
        // 一些靜音間隔(not in speech,不累積進 buf,但 total_samples_seen 持續走)
        for _ in 0..5 {
            assert!(v.push(&silent_chunk(), SILENT_DB).is_none());
        }
        // 段2 起點 = 目前 total_samples_seen(20 voice + 12 silence + 5 silence)
        let offset_before_seg2 = (20 + 12 + 5) * CHUNK as u64;
        for _ in 0..20 {
            v.push(&voice_chunk(), VOICE_DB);
        }
        for _ in 0..11 {
            v.push(&silent_chunk(), SILENT_DB);
        }
        let seg2 = v.push(&silent_chunk(), SILENT_DB).unwrap();
        let expected_ms = offset_before_seg2 * 1000 / SAMPLE_RATE;
        assert_eq!(seg2.start_offset_ms, expected_ms);
    }

    #[test]
    fn flush_emits_remaining_speech() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..20 {
            v.push(&voice_chunk(), VOICE_DB);
        }
        let seg = v.flush();
        assert!(seg.is_some());
        assert_eq!(seg.unwrap().samples.len(), 20 * CHUNK);
    }

    #[test]
    fn flush_drops_too_short() {
        let mut v = VadChunker::new(cfg());
        for _ in 0..3 {
            v.push(&voice_chunk(), VOICE_DB); // 0.15s < min
        }
        assert!(v.flush().is_none());
    }
}
