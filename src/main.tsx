import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

function App() {
  return <div style={{ padding: 16 }}>Mori Meeting Recorder — scaffold</div>;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode><App /></StrictMode>
);
