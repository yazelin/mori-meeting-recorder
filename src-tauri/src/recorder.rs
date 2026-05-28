//! Session lifecycle orchestrator。組合 audio::open_capture + SessionStore + transcribe + exporter。

use crate::audio::{self, CaptureHandle, SourceKind};
use crate::exporter::{export, Exports, SessionMeta, TrackMeta};
use crate::session_store::{default_meetings_dir, new_session_id, SessionStore};
use crate::transcribe::{Segment};
use tauri::Emitter;
use chrono::{DateTime, Local};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
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
}

pub struct Recorder {
    pub active: Mutex<Option<ActiveSession>>,
    pub state: Mutex<State>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
            state: Mutex::new(State::Idle),
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
    /// 從 SignalMeter snapshot 算 TrackLevel。Idle / 無訊號時 signal=false,peak/rms = -120 dB。
    pub fn from_signal_meter(meter: &crate::audio::SignalMeter, now_unix_ms: u64) -> Self {
        let signal = meter.has_signal(now_unix_ms);
        if signal {
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

        let mut handles = Vec::new();
        for kind in [SourceKind::MeetingSystem, SourceKind::MicInternal] {
            let out = store.audio_path(kind);
            match audio::open_capture(kind, out) {
                Ok(h) => handles.push(h),
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
        });
        *self.state.lock().map_err(|e| e.to_string())? = State::Recording;

        // === VU meter 50ms emit loop ===
        // Recorder is Arc-singleton via OnceLock, so we clone the Arc and let
        // the spawned task hold its own ref. The task self-stops when state != Recording.
        let recorder = instance();
        let app_for_task = app.clone();
        tokio::spawn(async move {
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

        // 停 capture
        for h in &session.handles {
            h.stop_flag.store(true, Ordering::Relaxed);
        }
        for h in session.handles {
            let _ = h.writer_handle.join();
        }

        // 轉錄 — parallel
        let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio rt: {e}"))?;
        let segs_result: Result<(Vec<Segment>, Vec<Segment>), String> = rt.block_on(async {
            let sys_path = store.audio_path(SourceKind::MeetingSystem);
            let mic_path = store.audio_path(SourceKind::MicInternal);
            let sys_id = session_id.clone();
            let mic_id = session_id.clone();
            let (sys, mic) = tokio::join!(
                tokio::task::spawn_blocking(move || {
                    crate::transcribe::run_whisper(&sys_path, &sys_id, SourceKind::MeetingSystem)
                }),
                tokio::task::spawn_blocking(move || {
                    crate::transcribe::run_whisper(&mic_path, &mic_id, SourceKind::MicInternal)
                }),
            );
            Ok((
                sys.map_err(|e| format!("join sys: {e}"))?,
                mic.map_err(|e| format!("join mic: {e}"))?,
            ))
        });
        let (sys_segs, mic_segs) = segs_result?;

        // 寫 segments JSONL
        write_segments_jsonl(&store.segments_path(SourceKind::MeetingSystem), &sys_segs)?;
        write_segments_jsonl(&store.segments_path(SourceKind::MicInternal), &mic_segs)?;

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
        }
    }
}

fn write_segments_jsonl(path: &PathBuf, segs: &[Segment]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let lines: Vec<String> = segs
        .iter()
        .map(|s| serde_json::to_string(s).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    std::fs::write(path, lines.join("\n") + "\n").map_err(|e| format!("write segs: {e}"))?;
    Ok(())
}

pub static RECORDER: std::sync::OnceLock<Arc<Recorder>> = std::sync::OnceLock::new();

pub fn instance() -> Arc<Recorder> {
    RECORDER
        .get_or_init(|| Arc::new(Recorder::default()))
        .clone()
}
