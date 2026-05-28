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
  const [err, setErr] = useState<string | null>(null);

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
    setErr(null);
    try {
      if (status?.state === "recording") await invoke("recorder_stop");
      else await invoke("recorder_start");
    } catch (e: any) {
      setErr(String(e));
      console.error(e);
    }
  };

  const recState = status?.state ?? "idle";
  const isRecording = recState === "recording";
  const isTranscribing = recState === "transcribing";
  const dotClass = `capsule-dot ${recState}`;
  const statusLabel = isRecording ? "REC" : isTranscribing ? t("capsule.transcribing") : "idle";
  const statusClass = `capsule-status ${recState}`;

  return (
    <div className="capsule" data-tauri-drag-region>
      <span className={dotClass} data-tauri-drag-region />
      <span className="capsule-title" data-tauri-drag-region>Recorder</span>
      <span className={statusClass} data-tauri-drag-region>{statusLabel}</span>
      <span className="capsule-spacer" data-tauri-drag-region />
      <span className="capsule-time" data-tauri-drag-region>{fmt(status?.elapsed_secs ?? 0)}</span>
      <span className="signal-pills">
        <span className={`signal-pill ${status?.system_signal ? "on" : ""}`} title={t("capsule.system_pill")}>
          <span className="signal-pill-dot" />SYS
        </span>
        <span className={`signal-pill ${status?.mic_signal ? "on" : ""}`} title={t("capsule.mic_pill")}>
          <span className="signal-pill-dot" />MIC
        </span>
        {err && (
          <span className="signal-pill err" title={err}>
            ⚠
          </span>
        )}
      </span>
      <button
        className={`icon-btn ${isRecording ? "danger" : "primary"}`}
        onClick={onStartStop}
        disabled={isTranscribing}
        title={isRecording ? t("capsule.stop") : t("capsule.start")}
      >
        {isRecording ? "■" : "▶"}
      </button>
      <button className="icon-btn" onClick={onExpand} title={t("capsule.expand")}>
        ▾
      </button>
    </div>
  );
}
