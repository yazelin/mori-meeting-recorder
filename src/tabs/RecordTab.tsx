import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import TriangleIcon from "../components/icons/TriangleIcon";
import SquareIcon from "../components/icons/SquareIcon";
import SpinnerIcon from "../components/icons/SpinnerIcon";
import TrackPanel from "../components/TrackPanel";

type RecState = "idle" | "recording" | "transcribing";

interface TrackLevel { peak_db: number; rms_db: number; signal: boolean }
interface LevelsPayload { sys: TrackLevel; mic: TrackLevel }

type Status = {
  state: RecState;
  elapsed_secs: number;
  session_id: string | null;
  system_signal: boolean;
  mic_signal: boolean;
  levels: LevelsPayload | null;
  sys_pending: number;
  sys_done: number;
  mic_pending: number;
  mic_done: number;
};

const fmtElapsed = (s: number) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
};

export default function RecordTab() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status | null>(null);
  const [levels, setLevels] = useState<LevelsPayload | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // 本場主題 + 參與者 —— 錄音中就能填,存進該場 session(meeting-info.json),PR H 整理時拿來用。
  const [topic, setTopic] = useState("");
  const [participants, setParticipants] = useState("");
  const saveInfo = () => { invoke("set_meeting_info", { topic, participants }).catch(() => {}); };

  const recState: RecState = status?.state ?? "idle";

  const [mode, setMode] = useState<"online" | "in_person">("online");
  useEffect(() => {
    invoke<{ recording_mode?: string }>("get_config")
      .then((c) => { if (c?.recording_mode === "in_person") setMode("in_person"); })
      .catch(() => {});
  }, []);
  const changeMode = async (m: "online" | "in_person") => {
    if (recState !== "idle" || m === mode) return; // 錄音中鎖住
    try {
      const cfg = await invoke<Record<string, unknown>>("get_config");
      await invoke("set_config", { cfg: { ...cfg, recording_mode: m } });
      setMode(m);
    } catch (e) { console.error(e); }
  };

  // 語音輸入:點 mic → 錄麥克風;再點 → whisper 轉錄、把文字接到欄位(append,可再打字修)。
  const [voiceField, setVoiceField] = useState<null | "topic" | "participants">(null);
  const toggleVoice = async (field: "topic" | "participants") => {
    if (voiceField === field) {
      setVoiceField(null);
      try {
        const text = await invoke<string>("voice_input_stop");
        if (text) {
          if (field === "topic") {
            const nv = (topic ? topic + " " : "") + text;
            setTopic(nv);
            invoke("set_meeting_info", { topic: nv, participants }).catch(() => {});
          } else {
            const nv = (participants ? participants + "\n" : "") + text;
            setParticipants(nv);
            invoke("set_meeting_info", { topic, participants: nv }).catch(() => {});
          }
        }
      } catch (e) { console.error(e); }
    } else if (voiceField === null) {
      try { await invoke("voice_input_start"); setVoiceField(field); } catch (e) { console.error(e); }
    }
  };
  const voiceBtn = (field: "topic" | "participants") => (
    <button
      className={`mi-voice ${voiceField === field ? "rec" : ""}`}
      onClick={() => toggleVoice(field)}
      disabled={voiceField !== null && voiceField !== field}
      title={t(voiceField === field ? "record.voice_stop" : "record.voice_input")}
    >
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="9" y="2" width="6" height="11" rx="3" />
        <path d="M5 10v1a7 7 0 0 0 14 0v-1" />
        <line x1="12" y1="19" x2="12" y2="22" />
      </svg>
    </button>
  );

  // status polling (500ms) — 兼當 levels polling fallback
  useEffect(() => {
    const tick = async () => {
      try {
        const s = await invoke<Status>("recorder_status");
        setStatus(s);
        if (s.levels) setLevels(s.levels);
      } catch { /* ignore */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);

  // Tauri "levels" event subscription — 50ms tick when recording
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<LevelsPayload>("levels", (e) => setLevels(e.payload));
    })();
    return () => { unlisten?.(); };
  }, []);

  const onStartStop = async () => {
    setErr(null);
    try {
      if (recState === "recording") {
        await invoke("recorder_stop");
        // stop_session 完成 → 立刻 refetch + 清掉 levels(VU 不要繼續顯示 stale data),
        // 不等 500ms polling 下個 tick(中間 lock contention 可能讓 polling fail/吞)。
        setLevels(null);
        const s = await invoke<Status>("recorder_status");
        setStatus(s);
      } else if (recState === "idle") {
        await invoke("recorder_start");
        // session 建好後把已填的主題/參與者寫進去
        await invoke("set_meeting_info", { topic, participants }).catch(() => {});
      }
    } catch (e: any) {
      setErr(String(e));
      console.error(e);
    }
  };

  const statusLabel =
    recState === "recording"     ? "REC" :
    recState === "transcribing"  ? t("capsule.transcribing") :
                                   t("record.idle_label");
  const actionTitle =
    recState === "recording"     ? t("capsule.stop") :
    recState === "transcribing"  ? t("capsule.transcribing") :
                                   t("capsule.start");

  return (
    <div>
      <div className="callout">⚠ {t("record.warning")}</div>

      <div className="mode-switch" role="group" aria-label={t("record.mode_label")} style={{ display: "flex", gap: 8, margin: "10px 0" }}>
        <button
          className={`mmr-btn${mode === "online" ? " primary" : ""}`}
          onClick={() => changeMode("online")}
          disabled={recState !== "idle"}
        >{t("record.mode_online")}</button>
        <button
          className={`mmr-btn${mode === "in_person" ? " primary" : ""}`}
          onClick={() => changeMode("in_person")}
          disabled={recState !== "idle"}
        >{t("record.mode_in_person")}</button>
      </div>

      <div className="record-control-bar">
        <span className="control-status">
          <span className={`control-dot ${recState}`} />
          <span className={`control-label ${recState}`}>{statusLabel}</span>
          <span className="control-time">{fmtElapsed(status?.elapsed_secs ?? 0)}</span>
        </span>
        <button
          className="control-action"
          data-state={recState}
          onClick={onStartStop}
          disabled={recState === "transcribing"}
          title={actionTitle}
        >
          {recState === "idle"        && <TriangleIcon size={14} />}
          {recState === "recording"   && <SquareIcon   size={12} />}
          {recState === "transcribing" && <SpinnerIcon size={16} />}
        </button>
      </div>

      {mode === "in_person" ? (
        <TrackPanel
          kind="mic"
          label={t("record.room_pill")}
          sourceName={t("record.source_room")}
          level={levels?.mic ?? null}
          progress={{ done: status?.mic_done ?? 0, pending: status?.mic_pending ?? 0 }}
        />
      ) : (
        <>
          <TrackPanel
            kind="sys"
            label={t("capsule.system_pill")}
            sourceName={t("record.source_sys")}
            level={levels?.sys ?? null}
            progress={{ done: status?.sys_done ?? 0, pending: status?.sys_pending ?? 0 }}
          />
          <TrackPanel
            kind="mic"
            label={t("capsule.mic_pill")}
            sourceName={t("record.source_mic")}
            level={levels?.mic ?? null}
            progress={{ done: status?.mic_done ?? 0, pending: status?.mic_pending ?? 0 }}
          />
        </>
      )}

      <div className="meeting-info">
        <div className="mi-field">
          <label className="mi-label">{t("record.topic")}</label>
          <div className="mi-input-row">
            <input
              className="mi-input"
              value={topic}
              placeholder={t("record.topic_ph")}
              onChange={(e) => setTopic(e.target.value)}
              onBlur={saveInfo}
            />
            {voiceBtn("topic")}
          </div>
        </div>
        <div className="mi-field">
          <label className="mi-label">{t("record.participants")}</label>
          <div className="mi-input-row">
            <textarea
              className="mi-input mi-textarea"
              value={participants}
              placeholder={t("record.participants_ph")}
              onChange={(e) => setParticipants(e.target.value)}
              onBlur={saveInfo}
              rows={2}
            />
            {voiceBtn("participants")}
          </div>
        </div>
      </div>

      {err && (
        <div className="callout" style={{ marginTop: 12, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.30)", background: "rgba(255,99,99,0.08)" }}>
          ⚠ {err}
        </div>
      )}
    </div>
  );
}
