import { useState } from "react";
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

export default function ExpandedView({ onCollapse, liveSys, liveMic }: { onCollapse: () => void; liveSys: LiveSegment[]; liveMic: LiveSegment[] }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("record");

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
