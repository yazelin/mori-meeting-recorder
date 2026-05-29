// src/components/TrackPanel.tsx
//
// 一張 Record tab 軌道卡:label + dot + bars icon / VU meter / dB readout / source name。
// 卡片底列右側放本軌的轉錄進度(已轉 N 段 · 轉錄中…),跟該軌同框,不另開一塊。

import { useTranslation } from "react-i18next";
import BarsIcon from "./icons/BarsIcon";
import VuMeter from "./VuMeter";

type Kind = "sys" | "mic";

interface Props {
  kind: Kind;
  label: string;
  sourceName: string;
  level: { peak_db: number; rms_db: number; signal: boolean } | null;
  progress?: { done: number; pending: number };
}

function fmtDb(db: number, signal: boolean): string {
  if (!signal) return "—";
  if (db <= -60) return "<-60 dB";
  return `${db.toFixed(0)} dB`;
}

export default function TrackPanel({ kind, label, sourceName, level, progress }: Props) {
  const { t } = useTranslation();
  const peakDb = level?.peak_db ?? -120;
  const rmsDb  = level?.rms_db ?? -120;
  const signal = level?.signal ?? false;
  // dot "active" = 錄音中且這軌真的有聲音(rmsDb > -40 dB,跟 capsule has_signal 同邏輯)。
  // 沒在錄音或這軌靜音 → dot 暗灰。
  const dotActive = signal && rmsDb > -40;
  return (
    <div className="track-panel">
      <div className="track-panel-row">
        <span className="track-panel-label">
          <span className={`track-panel-dot ${kind}${dotActive ? " active" : ""}`} />
          <BarsIcon size={10} />
          {label}
        </span>
        <div className="track-panel-meter-wrap">
          <VuMeter peakDb={peakDb} rmsDb={rmsDb} signal={signal} />
          <span className="track-panel-db">{fmtDb(rmsDb, signal)}</span>
        </div>
      </div>
      <div className="track-panel-foot">
        <span className="track-panel-source">{sourceName}</span>
        {progress && (
          <span className="track-panel-prog">
            {t("record.seg_done", { n: progress.done })}
            {progress.pending > 0 && <span className="tp-live"> · {t("capsule.transcribing")}</span>}
          </span>
        )}
      </div>
    </div>
  );
}
