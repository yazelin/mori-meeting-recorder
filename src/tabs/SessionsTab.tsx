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
      <p style={{ fontSize: 12, opacity: 0.7 }}>{t("sessions.hint")}</p>
      {sessions.length === 0 ? (
        <div style={{ opacity: 0.6 }}>{t("sessions.empty")}</div>
      ) : (
        <ul style={{ listStyle: "none", padding: 0 }}>
          {sessions.map((id) => (
            <li key={id} style={{ padding: "8px 0", borderBottom: "1px solid var(--c-border)" }}>
              <button className="mmr-btn" onClick={() => open(id)}>📁</button>
              <code style={{ marginLeft: 8 }}>{id}</code>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
