// src/components/icons/ChevronUpIcon.tsx
//
// Chevron pointing up — collapse. Mirror of ChevronDownIcon (expand/collapse pair).
// Canonical icon style: 24×24 viewBox, stroke=currentColor, width 2, round caps.

export default function ChevronUpIcon({ size = 16 }: { size?: number }) {
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
      <path d="m18 15-6-6-6 6" />
    </svg>
  );
}
