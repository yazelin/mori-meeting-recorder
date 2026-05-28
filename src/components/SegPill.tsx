// src/components/SegPill.tsx
//
// Sessions tab 卡片右上角的 segs 標籤 — public/internal 各一,顯示「public: 142 segs」。

type Tone = "public" | "internal";

interface Props {
  tone: Tone;
  count: number;
}

const LABEL: Record<Tone, string> = { public: "public", internal: "internal" };

export default function SegPill({ tone, count }: Props) {
  return (
    <span className="seg-pill" data-tone={tone}>
      {LABEL[tone]}: {count} segs
    </span>
  );
}
