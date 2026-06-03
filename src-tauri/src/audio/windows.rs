//! Windows:cpal WASAPI host。
//! - MicInternal:default input device
//! - MeetingSystem:WASAPI loopback(用 default_output_device 開 input stream,cpal 內部處理 loopback flag)

#![cfg(target_os = "windows")]

use super::{writer::TrackWriter, CaptureHandle, SignalMeter, SourceKind};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SampleRate, StreamConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const TARGET_RATE: u32 = 16_000;

pub fn pick_device(source: SourceKind) -> Result<Device, String> {
    let host = cpal::default_host();
    match source {
        SourceKind::MicInternal => host
            .default_input_device()
            .ok_or_else(|| "no default input device".into()),
        SourceKind::MeetingRoom => host
            .default_input_device()
            .ok_or_else(|| "no default input device".into()),
        SourceKind::MeetingSystem => host
            .default_output_device()
            .ok_or_else(|| "no default output device for loopback".into()),
    }
}

pub fn open_capture(
    source: SourceKind,
    out_path: PathBuf,
    vad_cfg: crate::audio::vad::VadConfig,
    pending: Arc<AtomicUsize>,
) -> Result<crate::audio::CaptureResult, String> {
    // caller 端要拿到的共享狀態:signal(VU meter 讀)、stop_flag(stop 時設)、speech_rx。
    let signal = Arc::new(Mutex::new(SignalMeter::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (speech_tx, speech_rx) = std::sync::mpsc::channel::<crate::audio::vad::SpeechSegment>();

    // Windows:cpal `Stream`(以及 `Device`)帶 WASAPI/COM thread affinity,是 `!Send`,
    // 不能像 Linux 那樣把 stream `move` 進別條 thread(Linux 的 Stream 是 Send 才編得過)。
    // 所以整個 cpal 生命週期(pick_device → build_input_stream → play → drop)都關在
    // 這一條 worker thread 內,絕不跨執行緒搬;build 階段的錯誤透過 ready channel 同步
    // 回報給 caller,讓 open_capture 仍能像以前一樣回傳 Result。
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    let stop_for_thread = stop_flag.clone();
    let signal_for_thread = signal.clone();
    let pending_for_thread = pending.clone();
    let speech_tx_thread = speech_tx.clone();
    // caller 只持有 rx;原始 tx drop 掉,確保最後一個 sender 消失時 worker 的 recv 會收到關閉。
    drop(speech_tx);

    let writer_thread = std::thread::spawn(move || {
        // ── cpal 物件全部在這條 thread 內建立 / 持有 / drop ──
        let device = match pick_device(source) {
            Ok(d) => d,
            Err(e) => {
                let _ = ready_tx.send(Err(e.clone()));
                return Err(e);
            }
        };
        let default_config = match source {
            SourceKind::MicInternal | SourceKind::MeetingRoom => device.default_input_config(),
            SourceKind::MeetingSystem => device.default_output_config(),
        };
        let default_config = match default_config {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("default config: {e}");
                let _ = ready_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        };
        let in_rate = default_config.sample_rate().0;
        let in_channels = default_config.channels();
        let sample_format = default_config.sample_format();

        let config = StreamConfig {
            channels: in_channels,
            sample_rate: SampleRate(in_rate),
            buffer_size: cpal::BufferSize::Default,
        };
        let resample_ratio = in_rate as f64 / TARGET_RATE as f64;

        let writer = Arc::new(Mutex::new(Some(match TrackWriter::create(&out_path) {
            Ok(w) => w,
            Err(e) => {
                let _ = ready_tx.send(Err(e.clone()));
                return Err(e);
            }
        })));
        let chunker = Arc::new(Mutex::new(crate::audio::vad::VadChunker::new(vad_cfg)));

        let err_fn = |e| eprintln!("audio stream error: {e}");

        let writer_cb = writer.clone();
        let signal_cb = signal_for_thread.clone();
        let chunker_cb = chunker.clone();
        let tx_cb = speech_tx_thread.clone();
        let pending_cb = pending_for_thread.clone();

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    handle_chunk_f32(data, in_channels, resample_ratio, &writer_cb, &signal_cb, &chunker_cb, &tx_cb, &pending_cb);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => {
                let writer_cb_i = writer_cb.clone();
                let signal_cb_i = signal_cb.clone();
                let chunker_cb_i = chunker_cb.clone();
                let tx_cb_i = tx_cb.clone();
                let pending_cb_i = pending_cb.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let f: Vec<f32> = data.iter().map(|&x| x as f32 / 32_768.0).collect();
                        handle_chunk_f32(&f, in_channels, resample_ratio, &writer_cb_i, &signal_cb_i, &chunker_cb_i, &tx_cb_i, &pending_cb_i);
                    },
                    err_fn,
                    None,
                )
            }
            other => {
                let msg = format!("unsupported sample format: {other:?}");
                let _ = ready_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        };
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("build_input_stream: {e}");
                let _ = ready_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        };
        if let Err(e) = stream.play() {
            let msg = format!("stream.play: {e}");
            let _ = ready_tx.send(Err(msg.clone()));
            return Err(msg);
        }

        // stream 已開始錄;通知 caller 成功。本 thread 之後持有 stream 直到 stop。
        let _ = ready_tx.send(Ok(()));

        while !stop_for_thread.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        drop(stream);
        // flush VadChunker 最後一段,再 drop tx → worker recv Err → 結束。
        if let Ok(mut c) = chunker.lock() {
            if let Some(seg) = c.flush() {
                pending_for_thread.fetch_add(1, Ordering::Relaxed);
                let _ = speech_tx_thread.send(seg);
            }
        }
        drop(speech_tx_thread);
        let mut guard = writer.lock().unwrap();
        if let Some(w) = guard.take() {
            let n = w.samples_written();
            w.finalize().map_err(|e| format!("finalize: {e}"))?;
            Ok(n)
        } else {
            Err("writer already finalized".into())
        }
    });

    // 等 worker 把 stream 建好且 play 成功(或回報建立失敗)再回傳,
    // 維持原本「open_capture 失敗就回 Err」的同步語意。
    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = writer_thread.join();
            return Err(e);
        }
        Err(_) => {
            let _ = writer_thread.join();
            return Err("capture thread exited before signaling ready".into());
        }
    }

    Ok((
        CaptureHandle {
            source,
            writer_handle: writer_thread,
            signal,
            stop_flag,
        },
        speech_rx,
    ))
}

#[allow(clippy::too_many_arguments)]
fn handle_chunk_f32(
    samples: &[f32],
    in_channels: u16,
    resample_ratio: f64,
    writer: &Arc<Mutex<Option<TrackWriter>>>,
    signal: &Arc<Mutex<SignalMeter>>,
    chunker: &Arc<Mutex<crate::audio::vad::VadChunker>>,
    speech_tx: &std::sync::mpsc::Sender<crate::audio::vad::SpeechSegment>,
    pending: &Arc<AtomicUsize>,
) {
    let mono: Vec<f32> = if in_channels == 1 {
        samples.to_vec()
    } else {
        samples
            .chunks(in_channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    let mut out_i16: Vec<i16> =
        Vec::with_capacity((mono.len() as f64 / resample_ratio) as usize + 1);
    let mut idx = 0.0_f64;
    while (idx as usize) < mono.len() {
        let v = mono[idx as usize].clamp(-1.0, 1.0);
        out_i16.push((v * 32_767.0) as i16);
        idx += resample_ratio;
    }

    if !out_i16.is_empty() {
        let (peak_db_raw, rms_db_raw) = crate::audio::levels::compute_levels(&mono);
        let now = chrono::Utc::now().timestamp_millis() as u64;
        if let Ok(mut s) = signal.lock() {
            // Smoothing same as Linux:fast attack, slow release(30 dB/秒)。詳細註解在 linux.rs。
            let dt_ms = now.saturating_sub(s.last_sample_at_unix_ms).clamp(1, 500) as f32;
            s.peak_rms_db = crate::audio::levels::smooth_db(s.peak_rms_db, rms_db_raw, 30.0, dt_ms);
            s.peak_db = crate::audio::levels::smooth_db(s.peak_db, peak_db_raw, 30.0, dt_ms);
            s.last_sample_at_unix_ms = now;
        }
        // VAD:用 raw rms 判切點(對齊 linux.rs);切出的段送 worker。
        if let Ok(mut c) = chunker.lock() {
            if let Some(seg) = c.push(&out_i16, rms_db_raw) {
                pending.fetch_add(1, Ordering::Relaxed);
                let _ = speech_tx.send(seg);
            }
        }
    }

    if let Ok(mut guard) = writer.lock() {
        if let Some(w) = guard.as_mut() {
            let _ = w.push_samples(&out_i16);
        }
    }
}
