//! WAV writer wrapper — hound 封裝。16kHz mono 16-bit PCM(對齊 whisper.cpp 原生輸入)。
//! Per-track 一個 WavWriter;recorder 對 system / mic-internal 各開一個。

use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// 固定的 WAV spec — 16kHz mono 16-bit signed PCM。
pub const WAV_SPEC: WavSpec = WavSpec {
    channels: 1,
    sample_rate: 16_000,
    bits_per_sample: 16,
    sample_format: SampleFormat::Int,
};

pub struct TrackWriter {
    inner: WavWriter<BufWriter<File>>,
    samples_written: u64,
}

impl TrackWriter {
    pub fn create(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir parent: {e}"))?;
        }
        let inner = WavWriter::create(path, WAV_SPEC)
            .map_err(|e| format!("WavWriter::create({}): {e}", path.display()))?;
        Ok(Self { inner, samples_written: 0 })
    }

    pub fn push_samples(&mut self, samples: &[i16]) -> Result<(), String> {
        for &s in samples {
            self.inner.write_sample(s).map_err(|e| e.to_string())?;
        }
        self.samples_written += samples.len() as u64;
        Ok(())
    }

    pub fn samples_written(&self) -> u64 {
        self.samples_written
    }

    pub fn finalize(self) -> Result<(), String> {
        self.inner.finalize().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::WavReader;
    use tempfile::TempDir;

    #[test]
    fn create_push_finalize_then_read_back_samples() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.wav");

        let mut w = TrackWriter::create(&path).unwrap();
        let signal: Vec<i16> = (0..1600).map(|i| (i as i16) * 10).collect();
        w.push_samples(&signal).unwrap();
        assert_eq!(w.samples_written(), 1600);
        w.finalize().unwrap();

        let mut r = WavReader::open(&path).unwrap();
        assert_eq!(r.spec().channels, 1);
        assert_eq!(r.spec().sample_rate, 16_000);
        assert_eq!(r.spec().bits_per_sample, 16);
        let read_back: Vec<i16> = r.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(read_back, signal);
    }

    #[test]
    fn create_makes_parent_dir() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("a/b/c/test.wav");
        let w = TrackWriter::create(&nested).unwrap();
        w.finalize().unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn empty_push_still_finalizes_to_valid_wav() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty.wav");
        let w = TrackWriter::create(&path).unwrap();
        w.finalize().unwrap();
        let r = WavReader::open(&path).unwrap();
        assert_eq!(r.len(), 0);
    }
}
