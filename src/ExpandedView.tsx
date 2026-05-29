import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import RecordTab from "./tabs/RecordTab";
import SessionsTab from "./tabs/SessionsTab";
import DepsTab from "./tabs/DepsTab";
import LiveTab from "./tabs/LiveTab";
import SettingsTab from "./tabs/SettingsTab";
import PeopleTab from "./tabs/PeopleTab";
import SignalPill from "./components/SignalPill";
import { type LiveSegment } from "./components/LiveColumn";

type Tab = "record" | "live" | "sessions" | "people" | "deps" | "settings";

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

  // 常駐錄音狀態 — 任何分頁(含 Live)的 header 都看得到狀態 + 時間 + SYS/MIC 訊號。
  type Status = { state: string; elapsed_secs: number; system_signal: boolean; mic_signal: boolean };
  const [rec, setRec] = useState<Status>({ state: "idle", elapsed_secs: 0, system_signal: false, mic_signal: false });
  const [captionsVisible, setCaptionsVisible] = useState(false);
  useEffect(() => {
    const tick = async () => {
      try { setRec(await invoke<Status>("recorder_status")); } catch { /* ignore */ }
      try { setCaptionsVisible(await invoke<boolean>("captions_visible")); } catch { /* ignore */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);
  const recLabel = rec.state === "recording" ? "REC" : rec.state === "transcribing" ? t("capsule.transcribing") : "idle";

  // 浮動字幕視窗開關。真實狀態以後端 captions_visible 為準(錄音 auto-show 也會反映),
  // CC 鈕跟著亮 → 不會「視窗開了但鈕沒亮」。
  const toggleCaptions = async () => {
    const next = !captionsVisible;
    setCaptionsVisible(next); // 樂觀更新,下個 poll 校正
    try { await invoke("set_captions", { visible: next }); } catch { /* ignore */ }
  };

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
        <button className={`tab-btn ${tab === "people" ? "active" : ""}`} onClick={() => setTab("people")}>
          {t("tabs.people")}
        </button>
        <button className={`tab-btn ${tab === "deps" ? "active" : ""}`} onClick={() => setTab("deps")}>
          {t("tabs.deps")}
        </button>
        <button className={`tab-btn ${tab === "settings" ? "active" : ""}`} onClick={() => setTab("settings")}>
          {t("tabs.settings")}
        </button>
        <span style={{ flex: 1 }} />
        <span className="exp-status">
          <SignalPill kind="sys" active={rec.system_signal} />
          <SignalPill kind="mic" active={rec.mic_signal} />
          <span className={`capsule-dot ${rec.state}`} />
          <span className={`exp-status-label ${rec.state}`}>{recLabel}</span>
          <span className="exp-status-time">{fmtElapsed(rec.elapsed_secs)}</span>
        </span>
        <button
          className={`icon-btn ${captionsVisible ? "active" : ""}`}
          onClick={toggleCaptions}
          title={captionsVisible ? "hide caption windows" : "show caption windows"}
        >CC</button>
        <button className="icon-btn" onClick={onCollapse} title="collapse">▴</button>
      </div>
      <div className="expanded-body" style={{ flex: 1 }}>
        {tab === "record" && <RecordTab />}
        {tab === "live" && <LiveTab sys={liveSys} mic={liveMic} />}
        {tab === "sessions" && <SessionsTab />}
        {tab === "people" && <PeopleTab />}
        {tab === "deps" && <DepsTab />}
        {tab === "settings" && <SettingsTab />}
      </div>
    </div>
  );
}
