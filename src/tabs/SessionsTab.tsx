import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

export default function SessionsTab() {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<string[]>([]);

  useEffect(() => {
    invoke<string[]>("list_sessions").then((s) => setSessions(s.sort().reverse())).catch(() => setSessions([]));
  }, []);

  const open = async (id: string) => { try { await invoke("open_session_dir", { sessionId: id }); } catch {} };

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("sessions.title")}</h3>
      <p style={{ fontSize: 11, color: "var(--text-dim)", marginBottom: 12 }}>{t("sessions.hint")}</p>
      {sessions.length === 0 ? (
        <div style={{ color: "var(--text-dim)" }}>{t("sessions.empty")}</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {sessions.map((id) => (
            <div key={id} className="session-row" onClick={() => open(id)}>
              <span style={{ fontSize: 14 }}>📁</span>
              <code style={{ fontSize: 11, color: "var(--text-secondary)" }}>{id}</code>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
