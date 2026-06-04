// src/components/Select.tsx
//
// 自繪下拉 —— 取代原生 <select>。原因(對齊 mori-desktop 的同款結論):Linux
// webkit2gtk 把 <select> 的下拉面板用 GTK widget 渲染,配色被 system GTK theme 鎖死,
// CSS / option{} 怎麼設都沒用(dark GTK theme → 白底白字看不到)。這個用 div 自繪 panel,
// 全部走 theme token,跟其餘 UI 一致。鍵盤可導航 + 點外面關閉。

import { useEffect, useRef, useState } from "react";
import ChevronDownIcon from "./icons/ChevronDownIcon";

export type SelectOption = { value: string; label: string };

export default function Select({
  value,
  options,
  onChange,
  disabled = false,
}: {
  value: string;
  options: SelectOption[];
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [hoverIdx, setHoverIdx] = useState(-1);
  const wrapRef = useRef<HTMLDivElement>(null);
  const current = options.find((o) => o.value === value);

  useEffect(() => {
    if (open) setHoverIdx(Math.max(0, options.findIndex((o) => o.value === value)));
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); setOpen(false); }
      else if (e.key === "ArrowDown") { e.preventDefault(); setHoverIdx((i) => Math.min(options.length - 1, i + 1)); }
      else if (e.key === "ArrowUp") { e.preventDefault(); setHoverIdx((i) => Math.max(0, i - 1)); }
      else if (e.key === "Enter") {
        e.preventDefault();
        if (hoverIdx >= 0 && hoverIdx < options.length) { onChange(options[hoverIdx].value); setOpen(false); }
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, hoverIdx, options, onChange]);

  return (
    <div ref={wrapRef} className={`mmr-select ${open ? "open" : ""}`}>
      <button type="button" className="mmr-select-trigger" disabled={disabled} onClick={() => { if (!disabled) setOpen((o) => !o); }}>
        <span>{current ? current.label : "—"}</span>
        <span className="mmr-select-caret" aria-hidden><ChevronDownIcon size={14} /></span>
      </button>
      {open && (
        <div className="mmr-select-panel">
          {options.map((opt, i) => (
            <button
              key={opt.value}
              type="button"
              className={`mmr-select-option ${opt.value === value ? "selected" : ""} ${i === hoverIdx ? "hover" : ""}`}
              onClick={() => { onChange(opt.value); setOpen(false); }}
              onMouseEnter={() => setHoverIdx(i)}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
