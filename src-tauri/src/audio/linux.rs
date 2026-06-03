//! Linux:libpulse client API(對齊 OBS linux-pulseaudio plugin)。
//! - MicInternal:default input source(`None` source name)
//! - MeetingSystem:第一個 `.monitor` source(透過 `pactl list short sources` 列出)
//!
//! 走 libpulse-simple sync API,blocking read 在 dedicated thread。
//! server 端做 resample / format conversion → 我們收到的就是 16kHz mono i16。

use super::{writer::TrackWriter, CaptureHandle, SignalMeter, SourceKind};
use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const TARGET_RATE: u32 = 16_000;
const CHUNK_MS: u64 = 50; // 50ms blocking read,讓 stop_flag check 不會卡太久
const CHUNK_SAMPLES: usize = (TARGET_RATE as u64 * CHUNK_MS / 1000) as usize; // 800 @16kHz
const CHUNK_BYTES: usize = CHUNK_SAMPLES * 2; // i16 = 2 bytes

/// 挑出符合 source 的 PulseAudio source name。
///
/// - MicInternal → `None`(讓 pulse 用 default input)。
/// - MeetingSystem → **default sink** 的 `.monitor`(用 `pactl get-default-sink`),
///   不是「第一個 .monitor」— PipeWire 機器常有多 sink(HDMI / analog / USB 麥克風),
///   每 sink 都有自己的 monitor,挑錯 sink 等於錄到 SUSPENDED 的 idle monitor → 全是
///   靜音。fallback chain:default sink.monitor → 第一個 RUNNING .monitor → 第一個 .monitor。
pub fn pick_source(source: SourceKind) -> Result<Option<String>, String> {
    match source {
        SourceKind::MicInternal => Ok(None),
        SourceKind::MeetingRoom => Ok(None),
        SourceKind::MeetingSystem => pick_system_monitor().map(Some),
    }
}

fn pick_system_monitor() -> Result<String, String> {
    // 1. `pactl get-default-sink` → default sink 名稱
    let default_sink = Command::new("pactl")
        .args(["get-default-sink"])
        .output()
        .map_err(|e| format!("spawn pactl: {e}(install pulseaudio-utils?)"))?;
    let default_monitor_name = if default_sink.status.success() {
        let name = String::from_utf8_lossy(&default_sink.stdout).trim().to_string();
        if name.is_empty() {
            None
        } else {
            Some(format!("{name}.monitor"))
        }
    } else {
        None
    };

    // 2. 列出所有 sources,做兩件事:(a) 確認 default monitor 存在;(b) 收集 fallback 候選。
    let out = Command::new("pactl")
        .args(["list", "short", "sources"])
        .output()
        .map_err(|e| format!("spawn pactl: {e}"))?;
    if !out.status.success() {
        return Err(format!("pactl list exited {}", out.status));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut all_monitors: Vec<&str> = Vec::new();
    let mut running_monitors: Vec<&str> = Vec::new();
    let mut default_monitor_exists = false;
    for line in stdout.lines() {
        // 格式:ID NAME MODULE FORMAT CHANNELS STATE
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 || !cols[1].ends_with(".monitor") {
            continue;
        }
        all_monitors.push(cols[1]);
        // 最後一欄是 state(IDLE / RUNNING / SUSPENDED)
        let state = cols.last().copied().unwrap_or("");
        if state.eq_ignore_ascii_case("RUNNING") {
            running_monitors.push(cols[1]);
        }
        if let Some(dm) = &default_monitor_name {
            if cols[1] == dm.as_str() {
                default_monitor_exists = true;
            }
        }
    }

    // 3. 優先序:default sink.monitor → 第一個 RUNNING → 第一個 ANY
    if default_monitor_exists {
        if let Some(dm) = default_monitor_name {
            eprintln!("system monitor: picked default sink monitor → {dm}");
            return Ok(dm);
        }
    }
    if let Some(rm) = running_monitors.first() {
        eprintln!("system monitor: default sink monitor missing, picked first RUNNING → {rm}");
        return Ok(rm.to_string());
    }
    if let Some(am) = all_monitors.first() {
        eprintln!("system monitor: no default / RUNNING, picked first ANY (may be SUSPENDED) → {am}");
        return Ok(am.to_string());
    }
    Err("no .monitor source — run `pactl load-module module-loopback` or check PipeWire config".into())
}

pub fn open_capture(
    source: SourceKind,
    out_path: PathBuf,
    vad_cfg: crate::audio::vad::VadConfig,
    pending: Arc<AtomicUsize>,
) -> Result<crate::audio::CaptureResult, String> {
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
    //
    // BufferAttr 不傳 → pulse 預設 latency ~350ms,會把 chunks 攢一批一次倒給 client,
    // audio thread 在 ~10ms 內 burst process 完然後 SignalMeter 停了 350ms 沒人寫,
    // 過了 500ms recency check → signal=false → VU bar 閃爍。
    // 顯指 fragsize = 1 chunk(50ms),強迫 pulse real-time delivery。其他欄 -1 = let pulse pick。
    let buffer_attr = BufferAttr {
        maxlength: (CHUNK_BYTES * 4) as u32, // 防 underrun;~200ms ring
        fragsize: CHUNK_BYTES as u32,        // 每次 deliver 1 個 50ms chunk
        tlength: u32::MAX,                   // playback only,record 不用
        prebuf: u32::MAX,                    // playback only
        minreq: u32::MAX,                    // playback only
    };
    let simple = Simple::new(
        None,                    // 預設 PA server(PipeWire 完全相容)
        "mori-meeting-recorder", // app name
        Direction::Record,
        source_name.as_deref(), // None = default input;Some("xxx.monitor") = system loopback
        match source {
            SourceKind::MicInternal | SourceKind::MeetingRoom => "mic-internal",
            SourceKind::MeetingSystem => "system-loopback",
        },
        &spec,
        None,                // 預設 channel map
        Some(&buffer_attr), // 50ms fragsize → 真實 20fps delivery
    )
    .map_err(|e| format!("pulse Simple::new: {e}"))?;

    let writer = Arc::new(Mutex::new(Some(TrackWriter::create(&out_path)?)));
    let signal = Arc::new(Mutex::new(SignalMeter::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));
    // VadChunker 切出的 speech 段透過此 channel 送給 transcribe worker。
    let (speech_tx, speech_rx) = std::sync::mpsc::channel::<crate::audio::vad::SpeechSegment>();

    // capture loop 在 dedicated thread(simple.read 是 blocking,thread 拿 simple ownership)
    let writer_for_thread = writer.clone();
    let signal_for_thread = signal.clone();
    let stop_for_thread = stop_flag.clone();
    let writer_handle = std::thread::spawn(move || -> Result<u64, String> {
        let mut chunker = crate::audio::vad::VadChunker::new(vad_cfg);
        let mut buf = vec![0u8; CHUNK_BYTES];
        while !stop_for_thread.load(Ordering::Relaxed) {
            match simple.read(&mut buf) {
                Ok(()) => {
                    let samples: Vec<i16> = buf
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    // Convert i16 samples to normalized f32 for levels::compute_levels (expects ±1.0 range)
                    let normalized: Vec<f32> = samples.iter().map(|&s| s as f32 / 32_768.0).collect();
                    let (peak_db_raw, rms_db_raw) = crate::audio::levels::compute_levels(&normalized);
                    let now = chrono::Utc::now().timestamp_millis() as u64;
                    if let Ok(mut s) = signal_for_thread.lock() {
                        // VU smoothing — fast attack, slow release at the data layer (NOT just CSS).
                        let dt_ms = now.saturating_sub(s.last_sample_at_unix_ms).clamp(1, 500) as f32;
                        s.peak_rms_db = crate::audio::levels::smooth_db(s.peak_rms_db, rms_db_raw, 30.0, dt_ms);
                        s.peak_db = crate::audio::levels::smooth_db(s.peak_db, peak_db_raw, 30.0, dt_ms);
                        s.last_sample_at_unix_ms = now;
                    }
                    // VAD:用 raw rms(smooth 前的真實瞬時值)判切點;切出的段送 worker。
                    // pending++ 在「送進 channel」時(不是 worker 收到時)→ 計到的是真實佇列深度。
                    if let Some(seg) = chunker.push(&samples, rms_db_raw) {
                        pending.fetch_add(1, Ordering::Relaxed);
                        let _ = speech_tx.send(seg);
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
        // stop_flag set → flush VadChunker 最後一段 → drop simple → finalize WAV
        if let Some(seg) = chunker.flush() {
            pending.fetch_add(1, Ordering::Relaxed);
            let _ = speech_tx.send(seg);
        }
        // speech_tx drop(離開 scope)→ worker recv() 收到 Err → worker loop 結束。
        drop(speech_tx);
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

    Ok((
        CaptureHandle {
            source,
            writer_handle,
            signal,
            stop_flag,
        },
        speech_rx,
    ))
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
