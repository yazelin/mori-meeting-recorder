// src/components/icons/TriangleIcon.tsx
//
// Filled play triangle, used by RecordButton in idle state.
// Sized 16×16 viewbox; consumer controls dimension via CSS.

export default function TriangleIcon({ size = 14 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M4 3 L13 8 L4 13 Z" />
    </svg>
  );
}
