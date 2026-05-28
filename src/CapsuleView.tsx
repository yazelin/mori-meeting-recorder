import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import SignalPill from "./components/SignalPill";
import RecordButton from "./components/RecordButton";
import ChevronDownIcon from "./components/icons/ChevronDownIcon";

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

// Tauri 2 + Wayland 下 `data-tauri-drag-region` 屬性不可靠(AgentPulse 也是這個結論)。
// 改 imperative:mousedown 左鍵 + 非 button 區 → 呼 startDragging。
const startDragOnMouseDown = (e: React.MouseEvent) => {
  const tgt = e.target as HTMLElement;
  console.log("[drag] mousedown", { button: e.button, tag: tgt.tagName, cls: tgt.className });
  if (e.button !== 0) return;
  if (tgt.closest("button")) { console.log("[drag] blocked by button"); return; }
  console.log("[drag] calling startDragging");
  getCurrentWindow().startDragging()
    .then(() => console.log("[drag] startDragging resolved"))
    .catch((e) => console.error("[drag] startDragging failed:", e));
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
  const dotClass = `capsule-dot ${recState}`;
  const statusLabel = recState === "recording" ? "REC" : recState === "transcribing" ? t("capsule.transcribing") : "idle";
  const statusClass = `capsule-status ${recState}`;

  return (
    <div className="capsule" data-state={recState} onMouseDown={startDragOnMouseDown}>
      <span className={dotClass} />
      <span className="capsule-title">Recorder</span>
      <span className={statusClass}>{statusLabel}</span>
      <span className="capsule-spacer" />
      <span className="capsule-time">{fmt(status?.elapsed_secs ?? 0)}</span>
      <span className="signal-pills">
        <SignalPill kind="sys" active={!!status?.system_signal} />
        <SignalPill kind="mic" active={!!status?.mic_signal} />
        {err && (
          <span className="signal-pill err" title={err}>⚠</span>
        )}
      </span>
      <RecordButton state={recState} onClick={onStartStop} />
      <button className="icon-btn" onClick={onExpand} title={t("capsule.expand")}>
        <ChevronDownIcon size={12} />
      </button>
    </div>
  );
}
