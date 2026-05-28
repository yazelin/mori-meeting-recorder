import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type Status = { state: "idle" | "recording" | "transcribing"; session_id: string | null; system_signal: boolean; mic_signal: boolean };

export default function RecordTab() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status | null>(null);
  const [lastSession, setLastSession] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    const tick = async () => {
      try { setStatus(await invoke<Status>("recorder_status")); } catch {}
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  const start = async () => {
    setErr(null);
    try { await invoke("recorder_start"); } catch (e: any) { setErr(String(e)); console.error(e); }
  };
  const stop = async () => {
    setErr(null);
    try {
      const id = await invoke<string>("recorder_stop");
      setLastSession(id);
    } catch (e: any) { setErr(String(e)); console.error(e); }
  };
  const openDir = async () => {
    if (lastSession) await invoke("open_session_dir", { sessionId: lastSession });
  };

  const isRecording = status?.state === "recording";
  const isTranscribing = status?.state === "transcribing";

  return (
    <div>
      <div className="callout">⚠ {t("record.warning")}</div>

      <div style={{ marginTop: 16, display: "flex", gap: 12 }}>
        {isRecording ? (
          <button className="mmr-btn danger lg" onClick={stop}>
            ■ {t("record.stop_button")}
          </button>
        ) : (
          <button className="mmr-btn primary lg" onClick={start} disabled={isTranscribing}>
            ▶ {t("record.start_button")}
          </button>
        )}
      </div>

      <div style={{ marginTop: 14, display: "flex", gap: 8 }}>
        <span className={`signal-pill ${status?.system_signal ? "on" : ""}`}>
          <span className="signal-pill-dot" /> {t("capsule.system_pill")}
        </span>
        <span className={`signal-pill ${status?.mic_signal ? "on" : ""}`}>
          <span className="signal-pill-dot" /> {t("capsule.mic_pill")}
        </span>
      </div>

      {err && (
        <div className="callout" style={{ marginTop: 12, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.30)", background: "rgba(255,99,99,0.08)" }}>
          ⚠ {err}
        </div>
      )}

      {isTranscribing && (
        <p style={{ marginTop: 16, color: "var(--text-secondary)" }}>{t("record.transcribing_hint")}</p>
      )}

      {lastSession && status?.state === "idle" && (
        <div style={{ marginTop: 16, display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ color: "var(--found-color)" }}>✓</span>
          <span>{t("record.done_title")}:</span>
          <code style={{ fontSize: 11, color: "var(--text-dim)" }}>{lastSession}</code>
          <button className="mmr-btn" onClick={openDir}>{t("record.open_folder")}</button>
        </div>
      )}
    </div>
  );
}
