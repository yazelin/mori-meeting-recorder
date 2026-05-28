import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type Status = { state: "idle" | "recording" | "transcribing"; session_id: string | null; system_signal: boolean; mic_signal: boolean };

export default function RecordTab() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status | null>(null);
  const [lastSession, setLastSession] = useState<string | null>(null);

  useEffect(() => {
    const tick = async () => {
      try { setStatus(await invoke<Status>("recorder_status")); } catch {}
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  const start = async () => { try { await invoke("recorder_start"); } catch (e) { console.error(e); } };
  const stop = async () => {
    try {
      const id = await invoke<string>("recorder_stop");
      setLastSession(id);
    } catch (e) { console.error(e); }
  };
  const openDir = async () => {
    if (lastSession) await invoke("open_session_dir", { sessionId: lastSession });
  };

  return (
    <div>
      <p style={{ background: "var(--c-pill-off)", padding: 10, borderRadius: 6, fontSize: 12 }}>
        ⚠ {t("record.warning")}
      </p>
      <div style={{ marginTop: 16, display: "flex", gap: 12 }}>
        {status?.state === "recording" ? (
          <button className="mmr-btn" onClick={stop} style={{ fontSize: 16, padding: "10px 24px" }}>
            ■ {t("record.stop_button")}
          </button>
        ) : (
          <button className="mmr-btn" onClick={start} disabled={status?.state === "transcribing"} style={{ fontSize: 16, padding: "10px 24px" }}>
            ▶ {t("record.start_button")}
          </button>
        )}
      </div>
      <div style={{ marginTop: 14, display: "flex", gap: 8 }}>
        <span className={`mmr-pill ${status?.system_signal ? "on" : ""}`}><span className="mmr-pill-dot" /> {t("capsule.system_pill")}</span>
        <span className={`mmr-pill ${status?.mic_signal ? "on" : ""}`}><span className="mmr-pill-dot" /> {t("capsule.mic_pill")}</span>
      </div>
      {status?.state === "transcribing" && <p style={{ marginTop: 16, opacity: 0.7 }}>{t("record.transcribing_hint")}</p>}
      {lastSession && status?.state === "idle" && (
        <div style={{ marginTop: 16 }}>
          ✓ {t("record.done_title")}: <code>{lastSession}</code>
          <button className="mmr-btn" onClick={openDir} style={{ marginLeft: 8 }}>{t("record.open_folder")}</button>
        </div>
      )}
    </div>
  );
}
