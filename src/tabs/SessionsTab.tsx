import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import MeetingCard from "../components/MeetingCard";
import SessionWorkspace from "./SessionWorkspace";

interface SessionSummary {
  id: string;
  started_at: string;
  duration_secs: number;
  public_segs: number;
  internal_segs: number;
  preview: string | null;
  topic: string;
  organized: boolean;
  corrupt: boolean;
}

export default function SessionsTab() {
  const { t } = useTranslation();
  const [summaries, setSummaries] = useState<SessionSummary[] | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);

  useEffect(() => {
    invoke<SessionSummary[]>("list_sessions_detailed")
      .then(setSummaries)
      .catch(() => setSummaries([]));
  }, []);

  const onOpen = async (id: string) => {
    try { await invoke("open_session_dir", { sessionId: id }); } catch {}
  };

  if (openId) {
    return <SessionWorkspace sessionId={openId} onBack={() => setOpenId(null)} />;
  }

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("sessions.title")}</h3>
      <p style={{ fontSize: 11, color: "var(--text-dim)", marginBottom: 12 }}>{t("sessions.hint")}</p>
      {summaries === null ? (
        <div style={{ color: "var(--text-dim)" }}>讀取中…</div>
      ) : summaries.length === 0 ? (
        <div style={{ color: "var(--text-dim)" }}>{t("sessions.empty")}</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {summaries.map((s) => (
            <MeetingCard key={s.id} summary={s} onOpen={onOpen} onWorkspace={s.corrupt ? undefined : setOpenId} />
          ))}
        </div>
      )}
    </div>
  );
}
