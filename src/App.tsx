import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import "./theme.css";
import "./i18n";
import CapsuleView from "./CapsuleView";
import ExpandedView from "./ExpandedView";
import { type LiveSegment } from "./components/LiveColumn";

export type Mode = "collapsed" | "expanded";

interface LiveSegmentEvent {
  track: "sys" | "mic";
  segment: { session_id: string; start_ms: number; text: string };
}

export default function App() {
  const [mode, setMode] = useState<Mode>("collapsed");

  // 即時字幕狀態住在 App —— 唯一橫跨「收合 ↔ 展開」與「分頁切換」都常駐的元件。
  // 若把 state + listener 放在 LiveTab,切走分頁/收合會卸載它:字幕被丟掉,
  // 而且不在 Live 分頁時冒出的段會永久收不到(listener 也跟著卸)。jsonl 檔不受影響,
  // 但即時畫面會漏。上提到這裡後 listener 跟著 App 整個生命週期常駐。
  const [liveSys, setLiveSys] = useState<LiveSegment[]>([]);
  const [liveMic, setLiveMic] = useState<LiveSegment[]>([]);
  const sessionRef = useRef<string | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<LiveSegmentEvent>("live-segment", (e) => {
        const { track, segment } = e.payload;
        const seg: LiveSegment = { start_ms: segment.start_ms, text: segment.text };
        // 新一場錄音(session_id 變了)→ 清掉上一場字幕,這段當新場第一句。
        if (segment.session_id !== sessionRef.current) {
          sessionRef.current = segment.session_id;
          setLiveSys(track === "sys" ? [seg] : []);
          setLiveMic(track === "mic" ? [seg] : []);
          return;
        }
        if (track === "sys") setLiveSys((p) => [...p, seg]);
        else setLiveMic((p) => [...p, seg]);
      });
    })();
    return () => { unlisten?.(); };
  }, []);

  // 新錄音開始 → Rust emit "live-reset" → 立刻清空上一場字幕(不等第一段)。
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen("live-reset", () => {
        sessionRef.current = null;
        setLiveSys([]);
        setLiveMic([]);
      });
    })();
    return () => { unlisten?.(); };
  }, []);

  // 浮動字幕視窗(caption-sys / caption-mic):錄音開始 show、停止 hide。
  // App 常駐,polling recorder_status 偵測 recording 轉換,只在轉換點動作(不每 tick 重複)。
  const wasRecording = useRef(false);
  useEffect(() => {
    const setCaptionVisible = async (visible: boolean) => {
      try { await invoke("set_captions", { visible }); } catch { /* ignore */ }
    };
    const tick = async () => {
      try {
        const s = await invoke<{ state: string }>("recorder_status");
        const rec = s.state === "recording";
        if (rec !== wasRecording.current) {
          wasRecording.current = rec;
          await setCaptionVisible(rec);
        }
      } catch { /* ignore */ }
    };
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);

  const switchMode = async (next: Mode) => {
    try { await invoke("set_window_mode", { mode: next }); } catch { /* ignore */ }
    setMode(next);
  };

  return mode === "collapsed"
    ? <CapsuleView onExpand={() => switchMode("expanded")} />
    : <ExpandedView onCollapse={() => switchMode("collapsed")} liveSys={liveSys} liveMic={liveMic} />;
}
