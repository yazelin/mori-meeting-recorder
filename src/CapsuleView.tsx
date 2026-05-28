export default function CapsuleView({ onExpand }: { onExpand: () => void }) {
  return <div onDoubleClick={onExpand} style={{ padding: 10 }}>capsule placeholder</div>;
}
