// src/components/TrackPanel.tsx
//
// 一張 Record tab 軌道卡:label + dot + bars icon / VU meter / dB readout / source name。
// 對應 mock 04 v2 中「會議音訊 · SYS」「內部麥克風 · MIC」兩列。

import BarsIcon from "./icons/BarsIcon";
import VuMeter from "./VuMeter";

type Kind = "sys" | "mic";

interface Props {
  kind: Kind;
  label: string;
  sourceName: string;
  level: { peak_db: number; rms_db: number; signal: boolean } | null;
}

function fmtDb(db: number, signal: boolean): string {
  if (!signal) return "—";
  if (db <= -60) return "<-60 dB";
  return `${db.toFixed(0)} dB`;
}

export default function TrackPanel({ kind, label, sourceName, level }: Props) {
  const peakDb = level?.peak_db ?? -120;
  const rmsDb  = level?.rms_db ?? -120;
  const signal = level?.signal ?? false;
  return (
    <div className="track-panel">
      <div className="track-panel-row">
        <span className="track-panel-label">
          <span className={`track-panel-dot ${kind}`} />
          <BarsIcon size={10} />
          {label}
        </span>
        <div className="track-panel-meter-wrap">
          <VuMeter peakDb={peakDb} rmsDb={rmsDb} signal={signal} />
          <span className="track-panel-db">{fmtDb(rmsDb, signal)}</span>
        </div>
      </div>
      <div className="track-panel-source">{sourceName}</div>
    </div>
  );
}
