import { useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import RecordTab from "./tabs/RecordTab";
import SessionsTab from "./tabs/SessionsTab";
import DepsTab from "./tabs/DepsTab";

type Tab = "record" | "sessions" | "deps";

// 同 CapsuleView:imperative startDragging on mousedown,Tauri 2 + Wayland 不能靠 data-tauri-drag-region。
const startDragOnMouseDown = (e: React.MouseEvent) => {
  if (e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (target.closest("button")) return;
  getCurrentWindow().startDragging().catch(() => {});
};

export default function ExpandedView({ onCollapse }: { onCollapse: () => void }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("record");

  return (
    <div id="view-expanded" style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
      <div className="expanded-header" onMouseDown={startDragOnMouseDown}>
        <button className={`tab-btn ${tab === "record" ? "active" : ""}`} onClick={() => setTab("record")}>
          {t("tabs.record")}
        </button>
        <button className={`tab-btn ${tab === "sessions" ? "active" : ""}`} onClick={() => setTab("sessions")}>
          {t("tabs.sessions")}
        </button>
        <button className={`tab-btn ${tab === "deps" ? "active" : ""}`} onClick={() => setTab("deps")}>
          {t("tabs.deps")}
        </button>
        <span style={{ flex: 1 }} />
        <button className="icon-btn" onClick={onCollapse} title="collapse">▴</button>
      </div>
      <div className="expanded-body" style={{ flex: 1 }}>
        {tab === "record" && <RecordTab />}
        {tab === "sessions" && <SessionsTab />}
        {tab === "deps" && <DepsTab />}
      </div>
    </div>
  );
}
