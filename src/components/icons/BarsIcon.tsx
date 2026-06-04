// src/components/icons/BarsIcon.tsx
//
// 3-bar equalizer (SignalPill sys/mic indicator). Color = currentColor.
// Canonical icon style: 24×24 viewBox, fill=currentColor, size via prop.

export default function BarsIcon({ size = 12 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect x="4" y="11" width="3.5" height="9" rx="1" />
      <rect x="10.25" y="5" width="3.5" height="15" rx="1" />
      <rect x="16.5" y="9" width="3.5" height="11" rx="1" />
    </svg>
  );
}
