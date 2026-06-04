// src/components/icons/CloseIcon.tsx
//
// X — close / quit. Replaces the ✕ glyph in capsule/expanded/caption headers.
// Canonical icon style: 24×24 viewBox, stroke=currentColor, width 2, round caps.

export default function CloseIcon({ size = 14 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}
