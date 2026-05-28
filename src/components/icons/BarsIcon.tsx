// src/components/icons/BarsIcon.tsx
//
// Tiny 3-vertical-bars equalizer icon for SignalPill.
// Color = currentColor — caller decides active/inactive.

export default function BarsIcon({ size = 10 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 10 10"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect x="1" y="5" width="1.6" height="4" rx="0.4" />
      <rect x="4.2" y="2" width="1.6" height="7" rx="0.4" />
      <rect x="7.4" y="4" width="1.6" height="5" rx="0.4" />
    </svg>
  );
}
