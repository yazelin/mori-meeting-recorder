// src/components/VuMeter.tsx
//
// 24-segment 水平 VU meter,給 Record tab 雙軌用。
//
// 模型(yazelin 2026-05-28 拍板):
//   - signal=true(錄音中,audio thread 活著)→ VU 一直看得到,大小聲變化
//   - signal=false(沒在錄音 / audio thread 停了)→ VU 全暗
//
// dB scale 從 -80 起算(不是 -60)— 室內 ambient ~-65 dB 也含進去,講話停頓
// 不會掉到全暗。signal=true 時保證最少 1 segment 亮(alive indicator)。

const TOTAL_SEGMENTS = 24;
const DB_MIN = -80;
const DB_MAX = 0;
const ALIVE_MIN_SEGMENTS = 1;

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
  const litSegs = Math.max(ALIVE_MIN_SEGMENTS, dbToSegmentCount(rmsDb));
  const peakIdx = Math.max(ALIVE_MIN_SEGMENTS, dbToSegmentCount(peakDb)) - 1;
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
