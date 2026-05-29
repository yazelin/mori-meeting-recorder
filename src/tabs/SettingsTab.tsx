// src/tabs/SettingsTab.tsx
//
// VAD 轉錄參數設定。get_config 讀,set_config 存。改了下次錄音生效。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import SettingField from "../components/SettingField";
import Select from "../components/Select";

interface RecorderConfig {
  silence_split_ms: number;
  silence_threshold_db: number;
  min_speech_secs: number;
  max_segment_secs: number;
  language: string;
  traditional: boolean;
  model: string;
}

const DEFAULTS: RecorderConfig = {
  silence_split_ms: 600,
  silence_threshold_db: -45,
  min_speech_secs: 0.5,
  max_segment_secs: 20,
  language: "zh",
  traditional: true,
  model: "small",
};

export default function SettingsTab() {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<RecorderConfig>(DEFAULTS);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    invoke<RecorderConfig>("get_config").then(setCfg).catch(() => setCfg(DEFAULTS));
  }, []);

  const save = async () => {
    try {
      await invoke("set_config", { cfg });
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) { console.error(e); }
  };

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("settings.title")}</h3>

      {/* Whisper model */}
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.model")}</span>
          <Select
            value={cfg.model}
            onChange={(v) => setCfg({ ...cfg, model: v })}
            options={[
              { value: "small", label: t("settings.model_small") },
              { value: "large-v3-turbo", label: t("settings.model_turbo") },
            ]}
          />
        </div>
        <div className="setting-field-hint">{t("settings.model_hint")}</div>
      </div>

      {/* Transcription language */}
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.language")}</span>
          <Select
            value={cfg.language}
            onChange={(v) => setCfg({ ...cfg, language: v })}
            options={[
              { value: "auto", label: t("settings.lang_auto") },
              { value: "zh", label: t("settings.lang_zh") },
              { value: "en", label: t("settings.lang_en") },
              { value: "ja", label: t("settings.lang_ja") },
            ]}
          />
        </div>
        <div className="setting-field-hint">{t("settings.language_hint")}</div>
      </div>

      {/* Traditional conversion toggle */}
      <div className="setting-field">
        <div className="setting-field-row">
          <span className="setting-field-label">{t("settings.traditional")}</span>
          <input
            type="checkbox"
            className="setting-field-checkbox"
            checked={cfg.traditional}
            onChange={(e) => setCfg({ ...cfg, traditional: e.target.checked })}
          />
        </div>
        <div className="setting-field-hint">{t("settings.traditional_hint")}</div>
      </div>

      <SettingField
        label={t("settings.silence_split")} hint={t("settings.silence_split_hint")}
        unit="ms" defaultLabel={t("settings.default", { v: 600 })}
        value={cfg.silence_split_ms} step={50}
        onChange={(v) => setCfg({ ...cfg, silence_split_ms: v })}
      />
      <SettingField
        label={t("settings.silence_threshold")} hint={t("settings.silence_threshold_hint")}
        unit="dB" defaultLabel={t("settings.default", { v: -45 })}
        value={cfg.silence_threshold_db} step={1}
        onChange={(v) => setCfg({ ...cfg, silence_threshold_db: v })}
      />
      <SettingField
        label={t("settings.min_speech")} hint={t("settings.min_speech_hint")}
        unit="s" defaultLabel={t("settings.default", { v: 0.5 })}
        value={cfg.min_speech_secs} step={0.1}
        onChange={(v) => setCfg({ ...cfg, min_speech_secs: v })}
      />
      <SettingField
        label={t("settings.max_segment")} hint={t("settings.max_segment_hint")}
        unit="s" defaultLabel={t("settings.default", { v: 20 })}
        value={cfg.max_segment_secs} step={1}
        onChange={(v) => setCfg({ ...cfg, max_segment_secs: v })}
      />
      <div style={{ display: "flex", gap: 8, marginTop: 14, alignItems: "center" }}>
        <button className="mmr-btn" onClick={() => setCfg(DEFAULTS)}>{t("settings.reset")}</button>
        <button className="mmr-btn primary" onClick={save}>{t("settings.save")}</button>
        {saved && <span style={{ color: "var(--found-color)", fontSize: 11 }}>{t("settings.saved")}</span>}
      </div>
    </div>
  );
}
