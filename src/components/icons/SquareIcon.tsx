// src/components/icons/SquareIcon.tsx
//
// Filled stop square, used by RecordButton in recording state.

export default function SquareIcon({ size = 12 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect x="3" y="3" width="10" height="10" rx="1.5" />
    </svg>
  );
}
