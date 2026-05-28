// src/components/icons/SpinnerIcon.tsx
//
// Rotating arc used by RecordButton in transcribing state.
// Wrap in <span class="spinner-rotate"> for animation(animate the wrapper,
// not the <svg> directly, to keep stroke vector clean).

export default function SpinnerIcon({ size = 14 }: { size?: number }) {
  return (
    <span className="spinner-rotate" aria-label="transcribing">
      <svg
        width={size}
        height={size}
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      >
        <path d="M8 2 A6 6 0 0 1 14 8" />
      </svg>
    </span>
  );
}
