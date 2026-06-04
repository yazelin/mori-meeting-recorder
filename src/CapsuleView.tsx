import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import SignalPill from "./components/SignalPill";
import RecordButton from "./components/RecordButton";
import ChevronDownIcon from "./components/icons/ChevronDownIcon";
import CloseIcon from "./components/icons/CloseIcon";
import AlertIcon from "./components/icons/AlertIcon";

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
  if (e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (target.closest("button")) return;
  getCurrentWindow().startDragging().catch((e) => console.error("startDragging failed:", e));
};

export default function CapsuleView({ onExpand }: { onExpand: () => void }) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<RecorderStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [captionsVisible, setCaptionsVisible] = useState(false);

  useEffect(() => {
    const tick = async () => {
      try { setStatus(await invoke<RecorderStatus>("recorder_status")); }
      catch { /* */ }
      try { setCaptionsVisible(await invoke<boolean>("captions_visible")); }
      catch { /* */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);

  const onStartStop = async () => {
    setErr(null);
    try {
      if (status?.state === "recording") {
        await invoke("recorder_stop");
        // 立刻 refetch,不等 polling 下個 tick(stop_session brief lock 可能讓 polling fail 吞)。
        setStatus(await invoke<RecorderStatus>("recorder_status"));
      } else await invoke("recorder_start");
    } catch (e: any) {
      setErr(String(e));
      console.error(e);
    }
  };

  // 浮動字幕視窗開關 — 跟 ExpandedView 的 CC 共用後端 captions_visible/set_captions,狀態同步。
  const toggleCaptions = async () => {
    const next = !captionsVisible;
    setCaptionsVisible(next); // 樂觀更新,下個 poll 校正
    try { await invoke("set_captions", { visible: next }); } catch { /* ignore */ }
  };

  // 結束整個 app — 膠囊也能關(不用先展開)。用 ExpandedView 同一個 quit_app 命令。
  const quitApp = async () => {
    try { await invoke("quit_app"); } catch { /* ignore */ }
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
          <span className="signal-pill err" title={err}><AlertIcon size={11} /></span>
        )}
      </span>
      <RecordButton state={recState} onClick={onStartStop} />
      <button
        className={`icon-btn ${captionsVisible ? "active" : ""}`}
        onClick={toggleCaptions}
        title={captionsVisible ? "hide caption windows" : "show caption windows"}
      >CC</button>
      <button className="icon-btn" onClick={onExpand} title={t("capsule.expand")}>
        <ChevronDownIcon size={16} />
      </button>
      <button className="icon-btn" onClick={quitApp} title="結束 / quit"><CloseIcon size={14} /></button>
    </div>
  );
}
