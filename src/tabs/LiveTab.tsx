// src/tabs/LiveTab.tsx
//
// 雙欄即時字幕。listen "live-segment" event,依 track 分流到 sys / mic 欄。

import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import LiveColumn, { type LiveSegment } from "../components/LiveColumn";

interface LiveSegmentEvent {
  track: "sys" | "mic";
  segment: { start_ms: number; text: string };
}

export default function LiveTab() {
  const { t } = useTranslation();
  const [sys, setSys] = useState<LiveSegment[]>([]);
  const [mic, setMic] = useState<LiveSegment[]>([]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<LiveSegmentEvent>("live-segment", (e) => {
        const seg = { start_ms: e.payload.segment.start_ms, text: e.payload.segment.text };
        if (e.payload.track === "sys") setSys((p) => [...p, seg]);
        else setMic((p) => [...p, seg]);
      });
    })();
    return () => { unlisten?.(); };
  }, []);

  const empty = sys.length === 0 && mic.length === 0;
  return (
    <div>
      {empty && <p style={{ color: "var(--text-dim)", fontSize: 12 }}>{t("live.empty")}</p>}
      <div className="live-cols">
        <LiveColumn title={t("live.col_sys")} segments={sys} />
        <LiveColumn title={t("live.col_mic")} segments={mic} />
      </div>
    </div>
  );
}
