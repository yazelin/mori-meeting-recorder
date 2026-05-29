import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import "./theme.css";
import "./i18n";
import App from "./App";
import CaptionWindow from "./CaptionWindow";

// 同一份 index.html 依視窗 label 分流:caption-* 是浮動字幕視窗,其餘是主 app。
// label 同步可取(getCurrentWebviewWindow().label 是屬性,不需 await)。
const label = getCurrentWebviewWindow().label;
const root = createRoot(document.getElementById("root")!);

if (label === "caption-sys" || label === "caption-mic") {
  root.render(
    <StrictMode>
      <CaptionWindow track={label === "caption-sys" ? "sys" : "mic"} />
    </StrictMode>
  );
} else {
  root.render(
    <StrictMode><App /></StrictMode>
  );
}
