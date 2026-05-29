// src/components/SettingField.tsx
//
// 一個參數列:label + 數字 input + 單位 + 預設值提示 + 一行說明。

interface Props {
  label: string;
  hint: string;
  unit: string;
  defaultLabel: string;
  value: number;
  step?: number;
  onChange: (v: number) => void;
}

export default function SettingField({ label, hint, unit, defaultLabel, value, step = 1, onChange }: Props) {
  return (
    <div className="setting-field">
      <div className="setting-field-row">
        <span className="setting-field-label">{label}</span>
        <input
          type="number"
          className="setting-field-input"
          value={value}
          step={step}
          onChange={(e) => onChange(parseFloat(e.target.value))}
        />
        <span className="setting-field-unit">{unit}</span>
        <span className="setting-field-default">{defaultLabel}</span>
      </div>
      <div className="setting-field-hint">{hint}</div>
    </div>
  );
}
