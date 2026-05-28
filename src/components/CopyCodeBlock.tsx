// src/components/CopyCodeBlock.tsx
//
// 帶 copy 按鈕的 code block — hover 顯示「複製」,點下去 navigator.clipboard 寫入,
// 按鈕短暫顯示「已複製 ✓」反饋。
//
// 在 Tauri 2 webview 中,navigator.clipboard.writeText 走標準 Web API,
// 不需要 Tauri clipboard plugin。

import { useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  code: string;
}

export default function CopyCodeBlock({ code }: Props) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      console.error("clipboard write failed:", e);
    }
  };

  return (
    <div className="code-block-wrap">
      <pre className="code-block">{code}</pre>
      <button
        type="button"
        className={`code-copy-btn${copied ? " copied" : ""}`}
        onClick={copy}
        title={t("deps.copy")}
      >
        {copied ? t("deps.copied") : t("deps.copy")}
      </button>
    </div>
  );
}
