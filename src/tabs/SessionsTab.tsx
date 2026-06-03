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
  const [query, setQuery] = useState("");
  const [fulltext, setFulltext] = useState(false);
  const [statusFilter, setStatusFilter] = useState<"all" | "organized" | "unorganized">("all");
  const [fulltextIds, setFulltextIds] = useState<Set<string> | null>(null);

  // 全文搜尋:開啟且有 query 時 debounce 呼後端;否則清掉(只走主題)。
  useEffect(() => {
    if (!fulltext || query.trim() === "") { setFulltextIds(null); return; }
    const id = setTimeout(async () => {
      try {
        const ids = await invoke<string[]>("search_sessions_fulltext", { query });
        setFulltextIds(new Set(ids));
      } catch { setFulltextIds(new Set()); }
    }, 300);
    return () => clearTimeout(id);
  }, [query, fulltext]);

  const toggleOrganized = async (id: string, next: boolean) => {
    try {
      await invoke("set_session_organized", { sessionId: id, organized: next });
      setSummaries((prev) => prev?.map((s) => (s.id === id ? { ...s, organized: next } : s)) ?? prev);
    } catch (e) { console.error(e); }
  };

  const visible = (summaries ?? []).filter((s) => {
    const q = query.trim().toLowerCase();
    const textOk = q === ""
      ? true
      : s.topic.toLowerCase().includes(q) || (fulltext && (fulltextIds?.has(s.id) ?? false));
    const statusOk = statusFilter === "all"
      ? true
      : statusFilter === "organized" ? s.organized : !s.organized;
    return textOk && statusOk;
  });

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
      <div style={{ display: "flex", flexWrap: "wrap", gap: 8, alignItems: "center", marginBottom: 10 }}>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜尋主題…"
          style={{ flex: 1, minWidth: 140, fontSize: 12, padding: "4px 8px" }}
        />
        <label style={{ fontSize: 11, color: "var(--text-secondary)", display: "flex", gap: 4, alignItems: "center" }}>
          <input type="checkbox" checked={fulltext} onChange={(e) => setFulltext(e.target.checked)} />
          含逐字稿內文
        </label>
        {(["all", "organized", "unorganized"] as const).map((v) => (
          <button
            key={v}
            className={`mmr-btn${statusFilter === v ? " primary" : ""}`}
            style={{ fontSize: 11, padding: "3px 8px" }}
            onClick={() => setStatusFilter(v)}
          >{v === "all" ? "全部" : v === "organized" ? "已整理" : "未整理"}</button>
        ))}
      </div>
      {summaries === null ? (
        <div style={{ color: "var(--text-dim)" }}>讀取中…</div>
      ) : summaries.length === 0 ? (
        <div style={{ color: "var(--text-dim)" }}>{t("sessions.empty")}</div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column" }}>
          {visible.map((s) => (
            <MeetingCard key={s.id} summary={s} onOpen={onOpen} onWorkspace={s.corrupt ? undefined : setOpenId} onToggleOrganized={s.corrupt ? undefined : toggleOrganized} />
          ))}
        </div>
      )}
    </div>
  );
}
