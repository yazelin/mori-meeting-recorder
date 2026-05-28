// src/components/icons/ChevronDownIcon.tsx
//
// 收合 / expand 鍵的箭頭。需要往上指時 caller 自己加 CSS transform: rotate(180deg)
// 或用 ChevronUpIcon(此 PR 不需,ExpandedView 那顆暫不動)。

export default function ChevronDownIcon({ size = 12 }: { size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M3 4.5 L6 7.5 L9 4.5" />
    </svg>
  );
}
