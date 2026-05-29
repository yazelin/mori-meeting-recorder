import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import RecordTab from "./tabs/RecordTab";
import SessionsTab from "./tabs/SessionsTab";
import DepsTab from "./tabs/DepsTab";
import LiveTab from "./tabs/LiveTab";
import SettingsTab from "./tabs/SettingsTab";
import { type LiveSegment } from "./components/LiveColumn";

type Tab = "record" | "live" | "sessions" | "deps" | "settings";

// 同 CapsuleView:imperative startDragging on mousedown,Tauri 2 + Wayland 不能靠 data-tauri-drag-region。
const startDragOnMouseDown = (e: React.MouseEvent) => {
  if (e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (target.closest("button")) return;
  getCurrentWindow().startDragging().catch(() => {});
};

const fmtElapsed = (s: number) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
};

export default function ExpandedView({ onCollapse, liveSys, liveMic }: { onCollapse: () => void; liveSys: LiveSegment[]; liveMic: LiveSegment[] }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("record");

  // 常駐錄音狀態 — 任何分頁(含 Live)的 header 都看得到狀態 + 時間。
  const [rec, setRec] = useState<{ state: string; elapsed_secs: number }>({ state: "idle", elapsed_secs: 0 });
  useEffect(() => {
    const tick = async () => {
      try { setRec(await invoke<{ state: string; elapsed_secs: number }>("recorder_status")); }
      catch { /* ignore */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);
  const recLabel = rec.state === "recording" ? "REC" : rec.state === "transcribing" ? t("capsule.transcribing") : "idle";

  return (
    <div id="view-expanded" style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
      <div className="expanded-header" onMouseDown={startDragOnMouseDown}>
        <button className={`tab-btn ${tab === "record" ? "active" : ""}`} onClick={() => setTab("record")}>
          {t("tabs.record")}
        </button>
        <button className={`tab-btn ${tab === "live" ? "active" : ""}`} onClick={() => setTab("live")}>
          {t("tabs.live")}
        </button>
        <button className={`tab-btn ${tab === "sessions" ? "active" : ""}`} onClick={() => setTab("sessions")}>
          {t("tabs.sessions")}
        </button>
        <button className={`tab-btn ${tab === "deps" ? "active" : ""}`} onClick={() => setTab("deps")}>
          {t("tabs.deps")}
        </button>
        <button className={`tab-btn ${tab === "settings" ? "active" : ""}`} onClick={() => setTab("settings")}>
          {t("tabs.settings")}
        </button>
        <span style={{ flex: 1 }} />
        <span className="exp-status">
          <span className={`capsule-dot ${rec.state}`} />
          <span className={`exp-status-label ${rec.state}`}>{recLabel}</span>
          <span className="exp-status-time">{fmtElapsed(rec.elapsed_secs)}</span>
        </span>
        <button className="icon-btn" onClick={onCollapse} title="collapse">▴</button>
      </div>
      <div className="expanded-body" style={{ flex: 1 }}>
        {tab === "record" && <RecordTab />}
        {tab === "live" && <LiveTab sys={liveSys} mic={liveMic} />}
        {tab === "sessions" && <SessionsTab />}
        {tab === "deps" && <DepsTab />}
        {tab === "settings" && <SettingsTab />}
      </div>
    </div>
  );
}
