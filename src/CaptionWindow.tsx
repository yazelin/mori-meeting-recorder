// src/CaptionWindow.tsx
//
// 單軌浮動字幕視窗(caption-sys / caption-mic 各一,由 tauri.conf.json 靜態定義、
// 預設 visible:false + alwaysOnTop)。每個是獨立 webview,自己 listen "live-segment"
// 過濾自己的 track。錄音開始時 App 把它 show()、停止時 hide()。frameless,可拖到
// 螢幕任何位置(mousedown→startDragging,對齊 CapsuleView 的 Wayland imperative drag)。

import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useTranslation } from "react-i18next";

interface LiveSegmentEvent {
  track: "sys" | "mic";
  segment: { session_id: string; start_ms: number; text: string };
}
interface Line { start_ms: number; text: string; }

function fmtTs(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

const startDragOnMouseDown = (e: React.MouseEvent) => {
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest("button")) return;
  getCurrentWebviewWindow().startDragging().catch(() => {});
};

export default function CaptionWindow({ track }: { track: "sys" | "mic" }) {
  const { t } = useTranslation();
  const [lines, setLines] = useState<Line[]>([]);
  const sessionRef = useRef<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<LiveSegmentEvent>("live-segment", (e) => {
        if (e.payload.track !== track) return;
        const { segment } = e.payload;
        const line: Line = { start_ms: segment.start_ms, text: segment.text };
        // 新一場錄音 → 清掉上一場
        if (segment.session_id !== sessionRef.current) {
          sessionRef.current = segment.session_id;
          setLines([line]);
          return;
        }
        setLines((p) => [...p, line]);
      });
    })();
    return () => { unlisten?.(); };
  }, [track]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [lines.length]);

  const title = track === "sys" ? t("live.col_sys") : t("live.col_mic");

  return (
    <div className="cap-win">
      <div className="cap-win-bar" onMouseDown={startDragOnMouseDown}>
        <span className="cap-win-title">{title}</span>
        <span style={{ flex: 1 }} />
        <button
          className="cap-win-close"
          onClick={() => getCurrentWebviewWindow().hide().catch(() => {})}
          title="hide"
        >✕</button>
      </div>
      <div className="cap-win-body">
        {lines.length === 0 && <p className="cap-win-empty">{t("live.empty")}</p>}
        {lines.map((l, i) => (
          <div key={i} className="live-line">
            <span className="live-ts">{fmtTs(l.start_ms)}</span>
            <span className="live-text">{l.text}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
