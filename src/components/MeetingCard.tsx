// src/components/MeetingCard.tsx
//
// Sessions tab 一張會議卡。對應 mock 05 的版型。
// 點卡身或 ↗ 都會 open session folder;corrupt session 只顯示 id + 警告,不能 open。

import SegPill from "./SegPill";

interface SessionSummary {
  id: string;
  started_at: string;
  duration_secs: number;
  public_segs: number;
  internal_segs: number;
  preview: string | null;
  corrupt: boolean;
}

interface Props {
  summary: SessionSummary;
  onOpen: (id: string) => void;
  onWorkspace?: (id: string) => void;
}

function fmtStartedAt(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const yyyy = d.getFullYear();
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd} ${hh}:${mi}`;
}

function fmtDuration(secs: number): string {
  if (secs === 0) return "0s";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

export default function MeetingCard({ summary, onOpen, onWorkspace }: Props) {
  if (summary.corrupt) {
    return (
      <div className="meeting-card corrupt" onClick={(e) => e.stopPropagation()}>
        <div className="mc-body">
          <span className="mc-id">{summary.id}</span>
          <span className="mc-corrupt-tag">資料損毀(無 timeline.json)</span>
        </div>
        <div className="mc-pills" />
        <button className="mc-open" disabled title="無法開啟損毀的 session">↗</button>
      </div>
    );
  }

  const open = () => onOpen(summary.id);
  const openWorkspace = (e: React.MouseEvent) => {
    e.stopPropagation();
    onWorkspace?.(summary.id);
  };
  return (
    <div className="meeting-card" onClick={onWorkspace ? () => onWorkspace(summary.id) : open}>
      <div className="mc-body">
        <span className="mc-id">{summary.id}</span>
        <span className="mc-subtitle">
          {fmtStartedAt(summary.started_at)} · {fmtDuration(summary.duration_secs)}
        </span>
        {summary.preview ? (
          <span className="mc-preview">{summary.preview}</span>
        ) : (
          <span className="mc-preview" style={{ fontStyle: "italic", color: "var(--text-dim)" }}>(無公開內容)</span>
        )}
      </div>
      <div className="mc-pills">
        <SegPill tone="public"   count={summary.public_segs} />
        <SegPill tone="internal" count={summary.internal_segs} />
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 4, alignItems: "center" }}>
        {onWorkspace && (
          <button
            className="mmr-btn"
            style={{ fontSize: 11, padding: "3px 8px" }}
            onClick={openWorkspace}
            title="整理工作區"
          >整理</button>
        )}
        <button
          className="mc-open"
          onClick={(e) => { e.stopPropagation(); open(); }}
          title="開啟資料夾"
        >↗</button>
      </div>
    </div>
  );
}
