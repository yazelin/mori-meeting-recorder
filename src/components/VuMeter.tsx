// src/components/VuMeter.tsx
//
// 24-segment 水平 VU meter,給 Record tab 雙軌用。
// dB → segment count 線性 mapping:-60 → 0 segment,0 → 全亮 24 segment。
// 最右側 lit segment 上 peak 色(橘)。signal=false → 全暗(idle 視覺)。

const TOTAL_SEGMENTS = 24;
const DB_MIN = -60;
const DB_MAX = 0;

function dbToSegmentCount(db: number): number {
  if (db <= DB_MIN) return 0;
  if (db >= DB_MAX) return TOTAL_SEGMENTS;
  return Math.round(((db - DB_MIN) / (DB_MAX - DB_MIN)) * TOTAL_SEGMENTS);
}

interface Props {
  peakDb: number;
  rmsDb: number;
  signal: boolean;
}

export default function VuMeter({ peakDb, rmsDb, signal }: Props) {
  if (!signal) {
    return (
      <span className="vu-meter" aria-hidden="true">
        {Array.from({ length: TOTAL_SEGMENTS }).map((_, i) => (
          <span key={i} className="vu-seg" />
        ))}
      </span>
    );
  }
  const litSegs = dbToSegmentCount(rmsDb);
  const peakIdx = dbToSegmentCount(peakDb) - 1;
  return (
    <span className="vu-meter" aria-hidden="true">
      {Array.from({ length: TOTAL_SEGMENTS }).map((_, i) => {
        const cls =
          i === peakIdx && peakIdx >= 0 ? "vu-seg peak" :
          i < litSegs                    ? "vu-seg lit"  :
                                           "vu-seg";
        return <span key={i} className={cls} />;
      })}
    </span>
  );
}
