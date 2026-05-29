//! Session lifecycle orchestrator。組合 audio::open_capture + SessionStore + transcribe + exporter。

use crate::audio::{self, CaptureHandle, SourceKind};
use crate::exporter::{export, Exports, SessionMeta, TrackMeta};
use crate::session_store::{default_meetings_dir, new_session_id, SessionStore};
use crate::transcribe::{Segment};
use tauri::Emitter;
use chrono::{DateTime, Local};
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Recording,
    Transcribing,
}

pub struct ActiveSession {
    pub store: SessionStore,
    pub started_at: DateTime<Local>,
    pub handles: Vec<CaptureHandle>,
    pub workers: Vec<crate::transcribe::TranscribeWorker>,
}

/// per-track 轉錄進度。放 Recorder(singleton)而非 ActiveSession,這樣 stop 把 active
/// 拿走後、worker 還在 drain 佇列時,status() 仍讀得到「剩幾段」。
#[derive(Default)]
pub struct TrackProgress {
    pub pending: Arc<AtomicUsize>,
    pub done: Arc<AtomicUsize>,
}

pub struct Recorder {
    pub active: Mutex<Option<ActiveSession>>,
    pub state: Mutex<State>,
    pub sys_progress: TrackProgress,
    pub mic_progress: TrackProgress,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
            state: Mutex::new(State::Idle),
            sys_progress: TrackProgress::default(),
            mic_progress: TrackProgress::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackLevel {
    pub peak_db: f32,
    pub rms_db: f32,
    pub signal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LevelsPayload {
    pub sys: TrackLevel,
    pub mic: TrackLevel,
}

impl TrackLevel {
    /// 從 SignalMeter snapshot 算 TrackLevel。
    ///
    /// **signal 語義**:「audio thread 活著 + 最近 500ms 有 sample 送進來」,**不**看 dB 高低。
    /// VU meter 要能顯示低音量(-50 ~ -60 dB 的 mic 講話)的真實 level,如果用 `has_signal()`
    /// 的 -40 dB 閾值,小聲講話會被誤判 idle → VU 全暗。-40 dB 閾值只給膠囊小圓點(綠/灰)用。
    pub fn from_signal_meter(meter: &crate::audio::SignalMeter, now_unix_ms: u64) -> Self {
        let recent = now_unix_ms.saturating_sub(meter.last_sample_at_unix_ms) < 500;
        if recent {
            Self {
                peak_db: meter.peak_db,
                rms_db: meter.peak_rms_db,
                signal: true,
            }
        } else {
            Self { peak_db: -120.0, rms_db: -120.0, signal: false }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecorderStatus {
    pub state: State,
    pub elapsed_secs: u64,
    pub system_signal: bool,
    pub mic_signal: bool,
    pub session_id: Option<String>,
    pub levels: Option<LevelsPayload>,
    pub sys_pending: usize,
    pub sys_done: usize,
    pub mic_pending: usize,
    pub mic_done: usize,
}

impl Recorder {
    pub fn start_session(&self, app: tauri::AppHandle) -> Result<String, String> {
        let mut active_guard = self.active.lock().map_err(|e| e.to_string())?;
        if active_guard.is_some() {
            return Err("session already running".into());
        }
        let now = Local::now();
        let session_id = new_session_id(now);
        let store = SessionStore::create(&session_id, &default_meetings_dir())?;

        let cfg = crate::config::read_config();
        let language = cfg.language.clone();
        let traditional = cfg.traditional;
        // 重置 per-track 進度計數(上一場歸零)。
        for p in [&self.sys_progress, &self.mic_progress] {
            p.pending.store(0, Ordering::Relaxed);
            p.done.store(0, Ordering::Relaxed);
        }
        let vad_cfg = crate::audio::vad::VadConfig {
            silence_split_ms: cfg.silence_split_ms,
            silence_threshold_db: cfg.silence_threshold_db,
            min_speech_secs: cfg.min_speech_secs,
            max_segment_secs: cfg.max_segment_secs,
        };

        let mut handles = Vec::new();
        let mut workers = Vec::new();
        for kind in [SourceKind::MeetingSystem, SourceKind::MicInternal] {
            let out = store.audio_path(kind);
            match audio::open_capture(kind, out, vad_cfg.clone()) {
                Ok((h, rx)) => {
                    // 前端 LiveTab 用 "sys"/"mic" 兩欄(不是 track_name 的 system/mic-internal)
                    let track = match kind {
                        SourceKind::MeetingSystem => "sys",
                        SourceKind::MicInternal => "mic",
                    };
                    let jsonl = store.segments_path(kind);
                    let sid = session_id.clone();
                    let app_for_worker = app.clone();
                    let prog = match kind {
                        SourceKind::MeetingSystem => &self.sys_progress,
                        SourceKind::MicInternal => &self.mic_progress,
                    };
                    let worker = crate::transcribe::spawn_transcribe_worker(
                        rx,
                        sid,
                        kind,
                        jsonl,
                        language.clone(),
                        traditional,
                        prog.pending.clone(),
                        prog.done.clone(),
                        move |segs| {
                            for s in segs {
                                let _ = app_for_worker.emit(
                                    "live-segment",
                                    serde_json::json!({ "track": track, "segment": s }),
                                );
                            }
                        },
                    );
                    handles.push(h);
                    workers.push(worker);
                }
                Err(e) => eprintln!("warning: open_capture {:?} failed: {e}", kind),
            }
        }
        if handles.is_empty() {
            return Err("no audio capture stream opened".into());
        }

        *active_guard = Some(ActiveSession {
            store,
            started_at: now,
            handles,
            workers,
        });
        *self.state.lock().map_err(|e| e.to_string())? = State::Recording;

        // 新一場錄音 → 廣播 live-reset,讓 in-tab Live 與兩個浮動字幕視窗立刻清空上一場字幕
        // (否則要等這場第一段轉完才靠 session_id 換掉,中間會看到上一場的稿,分不清新舊)。
        let _ = app.emit("live-reset", ());

        // === VU meter 50ms emit loop ===
        // Recorder is Arc-singleton via OnceLock, so we clone the Arc and let
        // the spawned task hold its own ref. The task self-stops when state != Recording.
        //
        // Use tauri::async_runtime::spawn (NOT tokio::spawn): sync Tauri commands
        // may run on threads without a tokio runtime context, where tokio::spawn
        // panics ("there is no reactor running") and crashes the process.
        let recorder = instance();
        let app_for_task = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
            // First tick fires immediately — that's fine, just emits initial silence
            loop {
                tick.tick().await;
                let status = recorder.status();
                if !matches!(status.state, State::Recording) {
                    break;
                }
                if let Some(levels) = status.levels.clone() {
                    let _ = app_for_task.emit("levels", levels);
                }
            }
        });

        Ok(session_id)
    }

    pub fn stop_session(&self) -> Result<String, String> {
        let mut active_guard = self.active.lock().map_err(|e| e.to_string())?;
        let session = active_guard.take().ok_or("no active session")?;
        drop(active_guard);

        *self.state.lock().map_err(|e| e.to_string())? = State::Transcribing;

        let session_id = session.store.session_id.clone();
        let store = session.store;
        let started_at = session.started_at;

        // 1. 停 capture:stop_flag → capture thread flush VadChunker → 送最後段 → drop Sender。
        for h in &session.handles {
            h.stop_flag.store(true, Ordering::Relaxed);
        }
        for h in session.handles {
            let _ = h.writer_handle.join();
        }
        // 2. capture thread 已 drop Sender → 各 worker recv() 收到 Err → loop 結束。
        //    join 等 worker 把佇列裡剩餘的段轉完(jsonl 已由 worker 即時 append)。
        for w in session.workers {
            let _ = w.handle.join();
        }
        // 3. 讀回兩軌 jsonl 彙整(不再 stop 時 batch 轉整檔)。
        let sys_segs = crate::transcribe::read_segments_jsonl(
            &store.segments_path(SourceKind::MeetingSystem),
        );
        let mic_segs = crate::transcribe::read_segments_jsonl(
            &store.segments_path(SourceKind::MicInternal),
        );

        // 匯出
        let stopped_at = Local::now();
        let all_segs: Vec<Segment> = sys_segs.iter().chain(mic_segs.iter()).cloned().collect();
        let meta = SessionMeta {
            schema_version: 1,
            session_id: session_id.clone(),
            started_at: started_at.to_rfc3339(),
            stopped_at: stopped_at.to_rfc3339(),
            duration_secs: (stopped_at - started_at).num_seconds().max(0) as u64,
            tracks: vec![
                TrackMeta {
                    name: "system".into(),
                    source_kind: "meeting_system".into(),
                    visibility: "public".into(),
                    audio_path: "audio/system.wav".into(),
                    transcript_path: "transcript/system.segments.jsonl".into(),
                    segment_count: sys_segs.len(),
                },
                TrackMeta {
                    name: "mic-internal".into(),
                    source_kind: "mic_internal".into(),
                    visibility: "internal".into(),
                    audio_path: "audio/mic-internal.wav".into(),
                    transcript_path: "transcript/mic-internal.segments.jsonl".into(),
                    segment_count: mic_segs.len(),
                },
            ],
            exports: Exports {
                public: "meeting.public.md".into(),
                internal: "meeting.internal.md".into(),
            },
        };
        let (pub_md, int_md, timeline) = export(&all_segs, &meta)?;
        std::fs::write(store.public_md_path(), pub_md).map_err(|e| format!("write public.md: {e}"))?;
        std::fs::write(store.internal_md_path(), int_md).map_err(|e| format!("write internal.md: {e}"))?;
        std::fs::write(store.timeline_path(), timeline).map_err(|e| format!("write timeline.json: {e}"))?;

        *self.state.lock().map_err(|e| e.to_string())? = State::Idle;
        Ok(session_id)
    }

    /// 把本場主題 / 參與者寫進當前 session 的 meeting-info.json(PR H 整理會議記錄時讀)。
    /// 沒在錄音(無 active session)則 no-op —— 開錄後 RecordTab 會再呼一次把已填的值寫進去。
    pub fn set_meeting_info(&self, topic: String, participants: String) -> Result<(), String> {
        let active = self.active.lock().map_err(|e| e.to_string())?;
        if let Some(s) = active.as_ref() {
            let path = s.store.root.join("meeting-info.json");
            let json = serde_json::json!({ "topic": topic, "participants": participants });
            let body = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
            std::fs::write(&path, body).map_err(|e| format!("write meeting-info: {e}"))?;
        }
        Ok(())
    }

    pub fn status(&self) -> RecorderStatus {
        let state = *self.state.lock().unwrap_or_else(|e| e.into_inner());
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let (elapsed_secs, system_signal, mic_signal, session_id, levels) = if let Some(s) = active.as_ref() {
            let elapsed = (Local::now() - s.started_at).num_seconds().max(0) as u64;
            let sys = s
                .handles
                .iter()
                .find(|h| h.source == SourceKind::MeetingSystem)
                .map(|h| h.signal.lock().map(|sm| sm.has_signal(now_ms)).unwrap_or(false))
                .unwrap_or(false);
            let mic = s
                .handles
                .iter()
                .find(|h| h.source == SourceKind::MicInternal)
                .map(|h| h.signal.lock().map(|sm| sm.has_signal(now_ms)).unwrap_or(false))
                .unwrap_or(false);
            let sys_level = s.handles.iter()
                .find(|h| h.source == SourceKind::MeetingSystem)
                .and_then(|h| h.signal.lock().ok().map(|sm| TrackLevel::from_signal_meter(&sm, now_ms)))
                .unwrap_or(TrackLevel { peak_db: -120.0, rms_db: -120.0, signal: false });
            let mic_level = s.handles.iter()
                .find(|h| h.source == SourceKind::MicInternal)
                .and_then(|h| h.signal.lock().ok().map(|sm| TrackLevel::from_signal_meter(&sm, now_ms)))
                .unwrap_or(TrackLevel { peak_db: -120.0, rms_db: -120.0, signal: false });
            (elapsed, sys, mic, Some(s.store.session_id.clone()), Some(LevelsPayload { sys: sys_level, mic: mic_level }))
        } else {
            (0, false, false, None, None)
        };
        RecorderStatus {
            state,
            elapsed_secs,
            system_signal,
            mic_signal,
            session_id,
            levels,
            sys_pending: self.sys_progress.pending.load(Ordering::Relaxed),
            sys_done: self.sys_progress.done.load(Ordering::Relaxed),
            mic_pending: self.mic_progress.pending.load(Ordering::Relaxed),
            mic_done: self.mic_progress.done.load(Ordering::Relaxed),
        }
    }
}


pub static RECORDER: std::sync::OnceLock<Arc<Recorder>> = std::sync::OnceLock::new();

pub fn instance() -> Arc<Recorder> {
    RECORDER
        .get_or_init(|| Arc::new(Recorder::default()))
        .clone()
}
