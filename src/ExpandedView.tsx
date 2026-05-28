import { useState } from "react";
import { useTranslation } from "react-i18next";
import RecordTab from "./tabs/RecordTab";
import SessionsTab from "./tabs/SessionsTab";
import DepsTab from "./tabs/DepsTab";

type Tab = "record" | "sessions" | "deps";

export default function ExpandedView({ onCollapse }: { onCollapse: () => void }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("record");

  const TabBtn = ({ id, label }: { id: Tab; label: string }) => (
    <button
      className="mmr-btn"
      onClick={() => setTab(id)}
      style={{ borderColor: tab === id ? "var(--c-accent)" : "var(--c-border)" }}
    >
      {label}
    </button>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "10px 14px", borderBottom: "1px solid var(--c-border)" }}>
        <TabBtn id="record" label={t("tabs.record")} />
        <TabBtn id="sessions" label={t("tabs.sessions")} />
        <TabBtn id="deps" label={t("tabs.deps")} />
        <span style={{ flex: 1 }} />
        <button className="mmr-btn" onClick={onCollapse}>▴</button>
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: 14 }}>
        {tab === "record" && <RecordTab />}
        {tab === "sessions" && <SessionsTab />}
        {tab === "deps" && <DepsTab />}
      </div>
    </div>
  );
}
