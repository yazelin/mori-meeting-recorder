// src/components/icons/SpinnerIcon.tsx
//
// Rotating arc — loading / transcribing / in-progress. The wrapper <span> carries
// the `spinner-rotate` animation (animate the wrapper, keep the stroke crisp).
// Accepts optional style/className so inline spinners can keep their spacing.
// Canonical icon style: 24×24 viewBox, stroke=currentColor, width 2, round caps.

import { type CSSProperties } from "react";

export default function SpinnerIcon({
  size = 14,
  style,
  className,
}: {
  size?: number;
  style?: CSSProperties;
  className?: string;
}) {
  return (
    <span
      className={`spinner-rotate${className ? ` ${className}` : ""}`}
      style={style}
      aria-label="loading"
    >
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
        <path d="M21 12a9 9 0 1 1-6.219-8.56" />
      </svg>
    </span>
  );
}
