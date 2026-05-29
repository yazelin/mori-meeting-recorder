//! Session lifecycle orchestrator。組合 audio::open_capture + SessionStore + transcribe + exporter。

use crate::audio::{self, CaptureHandle, SourceKind};
use crate::exporter::{export, Exports, SessionMeta, TrackMeta};
use crate::session_store::{default_meetings_dir, new_session_id, SessionStore};
use crate::transcribe::{Segment};
use tauri::Emitter;
use chrono::{DateTime, Local};
use serde::Serialize;
use std::path::PathBuf;
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
    /// 開錄當時的 whisper 模型(config.model),停止彙整時記進 timeline.json。
    pub transcribe_model: String,
}

/// 語音輸入(主題/參與者快速口述)用的獨立麥克風 capture — 跟會議錄音無關,只寫 temp WAV。
pub struct VoiceCapture {
    pub handle: CaptureHandle,
    pub temp_path: PathBuf,
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
    pub voice: Mutex<Option<VoiceCapture>>,
    pub enroll: Mutex<Option<VoiceCapture>>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
            state: Mutex::new(State::Idle),
            sys_progress: TrackProgress::default(),
            mic_progress: TrackProgress::default(),
            voice: Mutex::new(None),
            enroll: Mutex::new(None),
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
        // **全域鎖序 active→state**(start/stop/status 一律先 active 後 state)防 AB-BA deadlock:
        // 之前 start 持 active 再取 state、而 status 持 state 再取 active,前端每隔幾百 ms 的
        // status poll 撞上正在開場(持 active 數百 ms 跑 open_capture)就會互卡死。先取 active。
        let mut active_guard = self.active.lock().map_err(|e| e.to_string())?;
        if active_guard.is_some() {
            return Err("session already running".into());
        }
        // 上一場 stop 後是「非阻塞」收尾(背景 drain + 匯出),期間 state=Transcribing、active=None。
        // 此時開新場會 reset 共用進度計數、且舊收尾執行緒結束時把 state 打回 Idle → 打架,擋住。
        // 在持有 active 下驗 state(active→state):收尾執行緒設 Idle 也須先過 state lock,序一致。
        if *self.state.lock().map_err(|e| e.to_string())? == State::Transcribing {
            return Err("上一場仍在轉錄,待轉錄完成再開始新錄音".into());
        }
        let now = Local::now();
        let session_id = new_session_id(now);
        let store = SessionStore::create(&session_id, &default_meetings_dir())?;

        let cfg = crate::config::read_config();
        let language = cfg.language.clone();
        let traditional = cfg.traditional;
        let transcribe_model = cfg.model.clone(); // 記下這場用的模型,停止時寫進 timeline.json
        // engine=cli → 完全不碰 server。否則:若沒有可用共享 server,detached 拉起 supervisor(非阻塞),
        // worker 會在 warmup 期間每段重試 reachable_server() 直到接上(見 spawn_transcribe_worker)。
        let try_server = cfg.transcribe_engine != "cli";
        if try_server {
            autostart_whisper_server(&cfg.model);
        }
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
            let prog = match kind {
                SourceKind::MeetingSystem => &self.sys_progress,
                SourceKind::MicInternal => &self.mic_progress,
            };
            match audio::open_capture(kind, out, vad_cfg.clone(), prog.pending.clone()) {
                Ok((h, rx)) => {
                    // 前端 LiveTab 用 "sys"/"mic" 兩欄(不是 track_name 的 system/mic-internal)
                    let track = match kind {
                        SourceKind::MeetingSystem => "sys",
                        SourceKind::MicInternal => "mic",
                    };
                    let jsonl = store.segments_path(kind);
                    let sid = session_id.clone();
                    let app_for_worker = app.clone();
                    let worker = crate::transcribe::spawn_transcribe_worker(
                        rx,
                        sid,
                        kind,
                        jsonl,
                        language.clone(),
                        traditional,
                        prog.pending.clone(),
                        prog.done.clone(),
                        try_server,
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
            transcribe_model,
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

    /// 停止錄音 —— **非阻塞**:立刻停 capture、標記 Transcribing、把「等 worker 把佇列剩餘段
    /// 轉完 + 彙整 + 匯出」丟到背景執行緒,command 立刻返回 session_id。
    ///
    /// 之前這裡同步 `join` worker:turbo-on-CPU 每段都重載模型要好幾秒,佇列沒清完 stop 不返回
    /// → UI 看似卡死、且 state 卡 Transcribing 連帶開不了新場(「按了沒反應、也開不了新的」)。
    /// 改背景收尾後:UI 全程看得到「剩 N 段」倒數(進度計數在 Recorder 上、active 拿走也讀得到),
    /// 收尾完成才 → Idle。共享 whisper-server(快)會讓這段 drain 很短;cli fallback 則靠背景化不卡 UI。
    pub fn stop_session(&self) -> Result<String, String> {
        let mut active_guard = self.active.lock().map_err(|e| e.to_string())?;
        let session = active_guard.take().ok_or("no active session")?;
        // 持 active 下設 Transcribing(active→state,與 start/status 同鎖序),設完才放 active,
        // 避免「active 已 None 但 state 還 Recording」的空窗被 status 讀到。
        *self.state.lock().map_err(|e| e.to_string())? = State::Transcribing;
        drop(active_guard);

        let session_id = session.store.session_id.clone();

        // 先通知 capture 停(送最後一段 VAD → drop Sender,worker recv 才收得到 Err 收尾)。
        for h in &session.handles {
            h.stop_flag.store(true, Ordering::Relaxed);
        }

        // 背景收尾。catch_unwind 包住:即使 finalize panic(理論上不會)也保證 state 打回 Idle,
        // 否則永遠卡 Transcribing → 再也開不了新場(比卡死更糟,因為沒任何路徑能恢復)。
        let recorder = instance();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                recorder.finalize_session(session)
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("[stop] finalize failed: {e}"),
                Err(_) => eprintln!("[stop] finalize panicked; forcing state→Idle"),
            }
            if let Ok(mut st) = recorder.state.lock() {
                *st = State::Idle;
            }
        });

        Ok(session_id)
    }

    /// 背景收尾:join capture writer + worker(等佇列 drain 完)→ 讀回兩軌 jsonl 彙整 → 匯出三檔。
    /// 不碰 `state`(由 stop_session 的背景執行緒在結束時統一設 Idle,確保出錯也會回 Idle)。
    fn finalize_session(&self, session: ActiveSession) -> Result<(), String> {
        let store = session.store;
        let started_at = session.started_at;
        let transcribe_model = session.transcribe_model;
        let session_id = store.session_id.clone();

        // 1. capture thread flush VadChunker → 送最後段 → drop Sender。
        for h in session.handles {
            let _ = h.writer_handle.join();
        }
        // 2. Sender 已 drop → 各 worker recv() 收到 Err → loop 結束;join 等它把佇列剩餘段轉完。
        for w in session.workers {
            let _ = w.handle.join();
        }
        // 3. 讀回兩軌 jsonl 彙整(jsonl 已由 worker 即時 append,不再 stop 時 batch 轉整檔)。
        let sys_segs = crate::transcribe::read_segments_jsonl(
            &store.segments_path(SourceKind::MeetingSystem),
        );
        let mic_segs = crate::transcribe::read_segments_jsonl(
            &store.segments_path(SourceKind::MicInternal),
        );

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
            transcribe_model,                 // 這場用的 whisper 模型
            diarize_seg_model: None,          // 還沒分人(會後跑 diarize_session 才填)
            diarize_emb_model: None,
        };
        let (pub_md, int_md, timeline) = export(&all_segs, &meta, &[])?;
        std::fs::write(store.public_md_path(), pub_md).map_err(|e| format!("write public.md: {e}"))?;
        std::fs::write(store.internal_md_path(), int_md).map_err(|e| format!("write internal.md: {e}"))?;
        std::fs::write(store.timeline_path(), timeline).map_err(|e| format!("write timeline.json: {e}"))?;
        Ok(())
    }

    /// 語音輸入開始:獨立錄一小段麥克風(跟會議錄音無關)寫進 temp WAV。VAD 段的 receiver
    /// 直接丟掉(我們只要整段 WAV)。
    pub fn voice_input_start(&self) -> Result<(), String> {
        let mut guard = self.voice.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("voice input already running".into());
        }
        let temp_path = std::env::temp_dir().join("mori-voice-input.wav");
        let vad_cfg = crate::audio::vad::VadConfig {
            silence_split_ms: 600,
            silence_threshold_db: -45.0,
            min_speech_secs: 0.5,
            max_segment_secs: 60.0,
        };
        let dummy_pending = Arc::new(AtomicUsize::new(0));
        let (handle, _rx) =
            audio::open_capture(SourceKind::MicInternal, temp_path.clone(), vad_cfg, dummy_pending)?;
        *guard = Some(VoiceCapture { handle, temp_path });
        Ok(())
    }

    /// 語音輸入停止:停 capture → whisper 轉錄整段 temp WAV → 回文字(用 config 的語言/繁體)。
    pub fn voice_input_stop(&self) -> Result<String, String> {
        let vc = self
            .voice
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .ok_or("no voice input running")?;
        vc.handle.stop_flag.store(true, Ordering::Relaxed);
        let _ = vc.handle.writer_handle.join();
        let cfg = crate::config::read_config();
        // 單段語音輸入:engine=cli 不用 server,否則驗活一次。&mut 讓 server 失敗時 sticky 落 cli。
        let mut server = if cfg.transcribe_engine == "cli" {
            None
        } else {
            crate::whisper_discovery::reachable_server()
        };
        let segs = crate::transcribe::run_whisper(
            &vc.temp_path,
            "voice-input",
            SourceKind::MicInternal,
            &cfg.language,
            true, // 語音輸入(主題/參與者)一定轉台灣正體,不受會議稿的 traditional 勾選影響
            &mut server,
        );
        let _ = std::fs::remove_file(&vc.temp_path);
        let _ = std::fs::remove_file(vc.temp_path.with_extension("wav.json"));
        let text = segs
            .iter()
            .map(|s| s.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(text)
    }

    /// 聲紋錄音開始:把麥克風錄到 ~/.mori/voiceprints/enroll-temp.wav(不轉錄,僅供 embed)。
    /// 沿用 VoiceCapture 機制;用獨立 `enroll` 欄位,可與 voice_input 並存(但實際上不建議同時用)。
    pub fn enroll_record_start(&self) -> Result<(), String> {
        let mut guard = self.enroll.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("enroll recording already running".into());
        }
        let dir = dirs::home_dir().unwrap_or_default().join(".mori").join("voiceprints");
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir voiceprints: {e}"))?;
        let temp_path = dir.join("enroll-temp.wav");
        let vad_cfg = crate::audio::vad::VadConfig {
            silence_split_ms: 600,
            silence_threshold_db: -45.0,
            min_speech_secs: 0.5,
            max_segment_secs: 120.0,
        };
        let dummy_pending = Arc::new(AtomicUsize::new(0));
        let (handle, _rx) =
            audio::open_capture(SourceKind::MicInternal, temp_path.clone(), vad_cfg, dummy_pending)?;
        *guard = Some(VoiceCapture { handle, temp_path });
        Ok(())
    }

    /// 聲紋錄音停止:停 capture → 回 temp WAV 路徑(不刪不轉錄)。
    pub fn enroll_record_stop(&self) -> Result<PathBuf, String> {
        let vc = self
            .enroll
            .lock()
            .map_err(|e| e.to_string())?
            .take()
            .ok_or("no enroll recording running")?;
        vc.handle.stop_flag.store(true, Ordering::Relaxed);
        let _ = vc.handle.writer_handle.join();
        Ok(vc.temp_path)
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
        // 鎖序 active→state(與 start/stop 一致)防 AB-BA deadlock;兩者一起持有取得一致快照
        // (否則 state 已轉 Transcribing 但 active 還沒被拿走、或反之,會回出自相矛盾的狀態)。
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        let state = *self.state.lock().unwrap_or_else(|e| e.into_inner());
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


/// 找 mori-whisper-serve(跟 recorder 執行檔同目錄:dev 在 target/<profile>/,bundle 走 sidecar)。
fn whisper_serve_bin() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    #[cfg(windows)]
    let name = "mori-whisper-serve.exe";
    #[cfg(not(windows))]
    let name = "mori-whisper-serve";
    let p = dir.join(name);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// 若沒有可用的共享 whisper-server,detached spawn supervisor 把它拉起(fire-and-forget,不等 ready)。
/// **退出不關它** —— supervisor 自己 idle-reap(使用者選的生命週期:有人用就開、沒人用 10 分鐘自關)。
/// detached:Linux setsid / Windows DETACHED_PROCESS,讓它比 recorder 活得久。找不到 binary 就略過(走 cli)。
fn autostart_whisper_server(model: &str) {
    if crate::whisper_discovery::reachable_server().is_some() {
        return; // 已有活的共享 server,免動
    }
    let bin = match whisper_serve_bin() {
        Some(b) => b,
        None => {
            eprintln!("[recorder] mori-whisper-serve not found next to exe; skip autostart (use cli)");
            return;
        }
    };
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(["--model", model, "--idle-secs", "600"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid(); // 自成 session,recorder 關掉也不連帶收掉它
            Ok(())
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    match cmd.spawn() {
        Ok(_) => eprintln!("[recorder] autostarted mori-whisper-serve (model={model})"),
        Err(e) => eprintln!("[recorder] autostart mori-whisper-serve failed: {e}"),
    }
}

pub static RECORDER: std::sync::OnceLock<Arc<Recorder>> = std::sync::OnceLock::new();

pub fn instance() -> Arc<Recorder> {
    RECORDER
        .get_or_init(|| Arc::new(Recorder::default()))
        .clone()
}
