import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type RecorderStatus = {
  state: "idle" | "recording" | "transcribing";
  elapsed_secs: number;
  system_signal: boolean;
  mic_signal: boolean;
  session_id: string | null;
};

const fmt = (s: number) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
};

export default function CapsuleView({ onExpand }: { onExpand: () => void }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<RecorderStatus | null>(null);

  useEffect(() => {
    const tick = async () => {
      try { setStatus(await invoke<RecorderStatus>("recorder_status")); }
      catch { /* */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);

  const onStartStop = async () => {
    try {
      if (status?.state === "recording") await invoke("recorder_stop");
      else await invoke("recorder_start");
    } catch (e) { console.error(e); }
  };

  const isRecording = status?.state === "recording";
  const isTranscribing = status?.state === "transcribing";

  return (
    <div
      onDoubleClick={(e) => {
        const tag = (e.target as HTMLElement).tagName;
        if (tag === "BUTTON" || tag === "SPAN") return;
        onExpand();
      }}
      style={{
        display: "flex", alignItems: "center", gap: 8,
        height: 60, padding: "0 14px",
        userSelect: "none",
      }}
    >
      <div style={{ fontSize: 16, fontVariantNumeric: "tabular-nums", minWidth: 80 }}>
        {fmt(status?.elapsed_secs ?? 0)}
      </div>
      <div style={{ display: "flex", gap: 6, flex: 1 }}>
        <span className={`mmr-pill ${status?.system_signal ? "on" : ""}`}>
          <span className="mmr-pill-dot" /> {t("capsule.system_pill")}
        </span>
        <span className={`mmr-pill ${status?.mic_signal ? "on" : ""}`}>
          <span className="mmr-pill-dot" /> {t("capsule.mic_pill")}
        </span>
      </div>
      {isTranscribing ? (
        <span style={{ fontSize: 11, opacity: 0.7 }}>{t("capsule.transcribing")}</span>
      ) : (
        <button className="mmr-btn" onClick={onStartStop}>
          {isRecording ? t("capsule.stop") : t("capsule.start")}
        </button>
      )}
    </div>
  );
}
