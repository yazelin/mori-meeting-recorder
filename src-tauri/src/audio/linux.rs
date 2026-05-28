//! Linux:libpulse client API(對齊 OBS linux-pulseaudio plugin)。
//! - MicInternal:default input source(`None` source name)
//! - MeetingSystem:第一個 `.monitor` source(透過 `pactl list short sources` 列出)
//!
//! 走 libpulse-simple sync API,blocking read 在 dedicated thread。
//! server 端做 resample / format conversion → 我們收到的就是 16kHz mono i16。

use super::{writer::TrackWriter, CaptureHandle, SignalMeter, SourceKind};
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const TARGET_RATE: u32 = 16_000;
const CHUNK_MS: u64 = 50; // 50ms blocking read,讓 stop_flag check 不會卡太久
const CHUNK_SAMPLES: usize = (TARGET_RATE as u64 * CHUNK_MS / 1000) as usize; // 800 @16kHz
const CHUNK_BYTES: usize = CHUNK_SAMPLES * 2; // i16 = 2 bytes

/// 挑出符合 source 的 PulseAudio source name。
/// MicInternal → `None`(讓 pulse 用 default input)。
/// MeetingSystem → 第一個 `.monitor` 結尾 source(走 pactl 列)。
pub fn pick_source(source: SourceKind) -> Result<Option<String>, String> {
    match source {
        SourceKind::MicInternal => Ok(None),
        SourceKind::MeetingSystem => {
            let out = Command::new("pactl")
                .args(["list", "short", "sources"])
                .output()
                .map_err(|e| format!("spawn pactl: {e}(install pulseaudio-utils?)"))?;
            if !out.status.success() {
                return Err(format!("pactl exited {}", out.status));
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                // 格式:ID NAME MODULE FORMAT CHANNELS STATE
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 2 && cols[1].ends_with(".monitor") {
                    return Ok(Some(cols[1].to_string()));
                }
            }
            Err("no .monitor source — run `pactl load-module module-loopback` or check PipeWire config".into())
        }
    }
}

pub fn open_capture(source: SourceKind, out_path: PathBuf) -> Result<CaptureHandle, String> {
    let source_name = pick_source(source)?;
    let spec = Spec {
        format: Format::S16le,
        channels: 1,
        rate: TARGET_RATE,
    };
    if !spec.is_valid() {
        return Err("invalid pulse spec".into());
    }

    // libpulse 會 server-side 把 source 的 native format 降到我們要求的 16kHz mono i16 — 不用 resample
    let simple = Simple::new(
        None,                    // 預設 PA server(PipeWire 完全相容)
        "mori-meeting-recorder", // app name
        Direction::Record,
        source_name.as_deref(), // None = default input;Some("xxx.monitor") = system loopback
        match source {
            SourceKind::MicInternal => "mic-internal",
            SourceKind::MeetingSystem => "system-loopback",
        },
        &spec,
        None, // 預設 channel map
        None, // 預設 buffer attrs
    )
    .map_err(|e| format!("pulse Simple::new: {e}"))?;

    let writer = Arc::new(Mutex::new(Some(TrackWriter::create(&out_path)?)));
    let signal = Arc::new(Mutex::new(SignalMeter::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    // capture loop 在 dedicated thread(simple.read 是 blocking,thread 拿 simple ownership)
    let writer_for_thread = writer.clone();
    let signal_for_thread = signal.clone();
    let stop_for_thread = stop_flag.clone();
    let writer_handle = std::thread::spawn(move || -> Result<u64, String> {
        let mut buf = vec![0u8; CHUNK_BYTES];
        while !stop_for_thread.load(Ordering::Relaxed) {
            match simple.read(&mut buf) {
                Ok(()) => {
                    let samples: Vec<i16> = buf
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    // RMS → SignalMeter
                    let sumsq: f64 = samples.iter().map(|&x| (x as f64).powi(2)).sum();
                    let rms = (sumsq / samples.len() as f64).sqrt();
                    let rms_norm = rms / 32_768.0;
                    let db = if rms_norm > 0.0 {
                        20.0 * rms_norm.log10()
                    } else {
                        -120.0
                    };
                    let now = chrono::Utc::now().timestamp_millis() as u64;
                    if let Ok(mut s) = signal_for_thread.lock() {
                        s.peak_rms_db = db as f32;
                        s.last_sample_at_unix_ms = now;
                    }
                    // 寫 WAV
                    if let Ok(mut guard) = writer_for_thread.lock() {
                        if let Some(w) = guard.as_mut() {
                            let _ = w.push_samples(&samples);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("pulse read err: {e}");
                    break;
                }
            }
        }
        // stop_flag set → drop simple → finalize WAV
        drop(simple);
        let mut guard = writer_for_thread.lock().unwrap();
        if let Some(w) = guard.take() {
            let n = w.samples_written();
            w.finalize().map_err(|e| format!("finalize: {e}"))?;
            Ok(n)
        } else {
            Err("writer already finalized".into())
        }
    });

    Ok(CaptureHandle {
        source,
        writer_handle,
        signal,
        stop_flag,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_source_mic_internal_returns_none() {
        // MicInternal 不打 pactl,直接 Ok(None)
        assert_eq!(pick_source(SourceKind::MicInternal).unwrap(), None);
    }

    // MeetingSystem 的 pick_source 需要 pactl 在 PATH + PipeWire/PulseAudio 跑著。
    // CI 無此環境 → #[ignore]。yazelin 機器跑這個 should return Ok(Some("alsa_output.xxx.monitor"))。
    #[test]
    #[ignore]
    fn pick_source_meeting_system_returns_some_monitor() {
        let result = pick_source(SourceKind::MeetingSystem);
        match result {
            Ok(Some(name)) => assert!(name.ends_with(".monitor"), "got: {name}"),
            Ok(None) => panic!("expected Some(.monitor name), got None"),
            Err(e) => panic!("pick failed: {e}"),
        }
    }
}
