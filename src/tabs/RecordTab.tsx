import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import TriangleIcon from "../components/icons/TriangleIcon";
import SquareIcon from "../components/icons/SquareIcon";
import SpinnerIcon from "../components/icons/SpinnerIcon";
import TrackPanel from "../components/TrackPanel";

type RecState = "idle" | "recording" | "transcribing";

interface TrackLevel { peak_db: number; rms_db: number; signal: boolean }
interface LevelsPayload { sys: TrackLevel; mic: TrackLevel }

type Status = {
  state: RecState;
  elapsed_secs: number;
  session_id: string | null;
  system_signal: boolean;
  mic_signal: boolean;
  levels: LevelsPayload | null;
};

const fmtElapsed = (s: number) => {
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
};

export default function RecordTab() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<Status | null>(null);
  const [levels, setLevels] = useState<LevelsPayload | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // status polling (500ms) — 兼當 levels polling fallback
  useEffect(() => {
    const tick = async () => {
      try {
        const s = await invoke<Status>("recorder_status");
        setStatus(s);
        if (s.levels) setLevels(s.levels);
      } catch { /* ignore */ }
    };
    tick();
    const id = setInterval(tick, 500);
    return () => clearInterval(id);
  }, []);

  // Tauri "levels" event subscription — 50ms tick when recording
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    // Debug: 每秒印一次 listen "levels" 收到的次數,確認 emit/listen 真實頻率。
    // 若 ~20/s 表示 Tauri 端 OK;若 ~1-2/s 表示 webview 端 throttle 或 React batch。
    let count = 0;
    const logTimer = setInterval(() => {
      console.log(`[vu-debug] listen("levels") received ${count} events in last 1s`);
      count = 0;
    }, 1000);
    (async () => {
      unlisten = await listen<LevelsPayload>("levels", (e) => {
        count++;
        setLevels(e.payload);
      });
    })();
    return () => {
      unlisten?.();
      clearInterval(logTimer);
    };
  }, []);

  const recState: RecState = status?.state ?? "idle";
  const onStartStop = async () => {
    setErr(null);
    try {
      if (recState === "recording") await invoke("recorder_stop");
      else if (recState === "idle") await invoke("recorder_start");
    } catch (e: any) {
      setErr(String(e));
      console.error(e);
    }
  };

  const statusLabel =
    recState === "recording"     ? "REC" :
    recState === "transcribing"  ? t("capsule.transcribing") :
                                   t("record.idle_label");
  const actionTitle =
    recState === "recording"     ? t("capsule.stop") :
    recState === "transcribing"  ? t("capsule.transcribing") :
                                   t("capsule.start");

  return (
    <div>
      <div className="callout">⚠ {t("record.warning")}</div>

      <div className="record-control-bar">
        <span className="control-status">
          <span className={`control-dot ${recState}`} />
          <span className={`control-label ${recState}`}>{statusLabel}</span>
          <span className="control-time">{fmtElapsed(status?.elapsed_secs ?? 0)}</span>
        </span>
        <button
          className="control-action"
          data-state={recState}
          onClick={onStartStop}
          disabled={recState === "transcribing"}
          title={actionTitle}
        >
          {recState === "idle"        && <TriangleIcon size={14} />}
          {recState === "recording"   && <SquareIcon   size={12} />}
          {recState === "transcribing" && <SpinnerIcon size={16} />}
        </button>
      </div>

      <TrackPanel
        kind="sys"
        label={t("capsule.system_pill")}
        sourceName={t("record.source_sys")}
        level={levels?.sys ?? null}
      />
      <TrackPanel
        kind="mic"
        label={t("capsule.mic_pill")}
        sourceName={t("record.source_mic")}
        level={levels?.mic ?? null}
      />

      {err && (
        <div className="callout" style={{ marginTop: 12, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.30)", background: "rgba(255,99,99,0.08)" }}>
          ⚠ {err}
        </div>
      )}
    </div>
  );
}
