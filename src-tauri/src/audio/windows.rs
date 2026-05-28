//! Windows:cpal WASAPI host。
//! - MicInternal:default input device
//! - MeetingSystem:WASAPI loopback(用 default_output_device 開 input stream,cpal 內部處理 loopback flag)

#![cfg(target_os = "windows")]

use super::{writer::TrackWriter, CaptureHandle, SignalMeter, SourceKind};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, SampleRate, StreamConfig};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const TARGET_RATE: u32 = 16_000;

pub fn pick_device(source: SourceKind) -> Result<Device, String> {
    let host = cpal::default_host();
    match source {
        SourceKind::MicInternal => host
            .default_input_device()
            .ok_or_else(|| "no default input device".into()),
        SourceKind::MeetingSystem => host
            .default_output_device()
            .ok_or_else(|| "no default output device for loopback".into()),
    }
}

pub fn open_capture(source: SourceKind, out_path: PathBuf) -> Result<CaptureHandle, String> {
    let device = pick_device(source)?;
    let default_config = match source {
        SourceKind::MicInternal => device
            .default_input_config()
            .map_err(|e| format!("default_input_config: {e}"))?,
        SourceKind::MeetingSystem => device
            .default_output_config()
            .map_err(|e| format!("default_output_config (loopback): {e}"))?,
    };
    let in_rate = default_config.sample_rate().0;
    let in_channels = default_config.channels();
    let sample_format = default_config.sample_format();

    let config = StreamConfig {
        channels: in_channels,
        sample_rate: SampleRate(in_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let writer = Arc::new(Mutex::new(Some(TrackWriter::create(&out_path)?)));
    let signal = Arc::new(Mutex::new(SignalMeter::default()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let resample_ratio = in_rate as f64 / TARGET_RATE as f64;
    let err_fn = |e| eprintln!("audio stream error: {e}");

    let writer_cb = writer.clone();
    let signal_cb = signal.clone();

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                handle_chunk_f32(data, in_channels, resample_ratio, &writer_cb, &signal_cb);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => {
            let writer_cb_i = writer_cb.clone();
            let signal_cb_i = signal_cb.clone();
            device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f: Vec<f32> = data.iter().map(|&x| x as f32 / 32_768.0).collect();
                    handle_chunk_f32(&f, in_channels, resample_ratio, &writer_cb_i, &signal_cb_i);
                },
                err_fn,
                None,
            )
        }
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("build_input_stream: {e}"))?;

    stream.play().map_err(|e| format!("stream.play: {e}"))?;

    let stop_for_thread = stop_flag.clone();
    let writer_for_finalize = writer.clone();
    let writer_thread = std::thread::spawn(move || {
        while !stop_for_thread.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        drop(stream);
        let mut guard = writer_for_finalize.lock().unwrap();
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
        writer_handle: writer_thread,
        signal,
        stop_flag,
    })
}

fn handle_chunk_f32(
    samples: &[f32],
    in_channels: u16,
    resample_ratio: f64,
    writer: &Arc<Mutex<Option<TrackWriter>>>,
    signal: &Arc<Mutex<SignalMeter>>,
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
    }

    if let Ok(mut guard) = writer.lock() {
        if let Some(w) = guard.as_mut() {
            let _ = w.push_samples(&out_i16);
        }
    }
}
