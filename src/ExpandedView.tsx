export default function ExpandedView({ onCollapse }: { onCollapse: () => void }) {
  return (
    <div style={{ padding: 16 }}>
      <button className="mmr-btn" onClick={onCollapse}>▴ collapse</button>
      <p>expanded placeholder — 3 tabs in Task 12</p>
    </div>
  );
}
