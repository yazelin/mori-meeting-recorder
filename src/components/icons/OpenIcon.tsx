// src/components/icons/OpenIcon.tsx
//
// Arrow up-right — open folder / external. Replaces the ↗ glyph in MeetingCard.
// Canonical icon style: 24×24 viewBox, stroke=currentColor, width 2, round caps.

export default function OpenIcon({ size = 14 }: { size?: number }) {
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
      <path d="M7 17 17 7" />
      <path d="M7 7h10v10" />
    </svg>
  );
}
