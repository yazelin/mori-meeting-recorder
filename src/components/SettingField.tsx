// src/components/SettingField.tsx
//
// 一個參數列:label + 數字 stepper(− input +)+ 單位 + 預設值提示 + 一行說明。
// 數字用自繪 − / + 鈕 —— 原生 <input type=number> 的 spinner 在 Linux webkit2gtk 是
// GTK widget,配色管不到(白底白字看不到)。CSS 也隱藏了原生 spinner。

interface Props {
  label: string;
  hint: string;
  unit: string;
  defaultLabel: string;
  value: number;
  step?: number;
  onChange: (v: number) => void;
}

// 用 step 推小數位,消掉浮點噪音(0.1 + 0.2 = 0.30000004)。
function roundToStep(v: number, step: number): number {
  const decimals = (String(step).split(".")[1] || "").length;
  return Number(v.toFixed(decimals));
}

export default function SettingField({ label, hint, unit, defaultLabel, value, step = 1, onChange }: Props) {
  return (
    <div className="setting-field">
      <div className="setting-field-row">
        <span className="setting-field-label">{label}</span>
        <div className="setting-stepper">
          <button type="button" className="setting-stepper-btn" onClick={() => onChange(roundToStep(value - step, step))}>−</button>
          <input
            type="number"
            className="setting-field-input"
            value={value}
            step={step}
            onChange={(e) => onChange(parseFloat(e.target.value))}
          />
          <button type="button" className="setting-stepper-btn" onClick={() => onChange(roundToStep(value + step, step))}>+</button>
        </div>
        <span className="setting-field-unit">{unit}</span>
        <span className="setting-field-default">{defaultLabel}</span>
      </div>
      <div className="setting-field-hint">{hint}</div>
    </div>
  );
}
