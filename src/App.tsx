import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./theme.css";
import "./i18n";
import CapsuleView from "./CapsuleView";
import ExpandedView from "./ExpandedView";

export type Mode = "collapsed" | "expanded";

export default function App() {
  const [mode, setMode] = useState<Mode>("collapsed");

  const switchMode = async (next: Mode) => {
    try { await invoke("set_window_mode", { mode: next }); } catch { /* ignore */ }
    setMode(next);
  };

  return mode === "collapsed"
    ? <CapsuleView onExpand={() => switchMode("expanded")} />
    : <ExpandedView onCollapse={() => switchMode("collapsed")} />;
}
