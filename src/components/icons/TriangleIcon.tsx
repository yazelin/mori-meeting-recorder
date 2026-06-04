// src/components/icons/TriangleIcon.tsx
//
// Filled play triangle (RecordButton idle state).
// Canonical icon style: 24×24 viewBox, fill=currentColor, size via prop.

export default function TriangleIcon({ size = 14 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M8 5v14l11-7z" />
    </svg>
  );
}
