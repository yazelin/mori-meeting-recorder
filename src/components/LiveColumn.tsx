// src/components/LiveColumn.tsx
//
// 單欄即時字幕滾動(SYS or MIC)。每行時間戳 + 文字,新段從底長出 + auto-scroll。

import { useEffect, useRef } from "react";

export interface LiveSegment {
  start_ms: number;
  text: string;
}

function fmtTs(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

export default function LiveColumn({ title, segments }: { title: string; segments: LiveSegment[] }) {
  const bottomRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [segments.length]);
  return (
    <div className="live-col">
      <div className="live-col-title">{title}</div>
      <div className="live-col-body">
        {segments.map((s, i) => (
          <div key={i} className="live-line">
            <span className="live-ts">{fmtTs(s.start_ms)}</span>
            <span className="live-text">{s.text}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
