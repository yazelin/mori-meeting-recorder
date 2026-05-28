import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

type DepsCheck = {
  whisper_cli_ok: boolean;
  whisper_cli_path: string;
  model_ok: boolean;
  model_path: string;
};

const LINUX_CMD = "bash <(curl -fsSL https://raw.githubusercontent.com/yazelin/mori-meeting-recorder/main/scripts/install-whisper-linux.sh)";
const WINDOWS_CMD = "iwr https://raw.githubusercontent.com/yazelin/mori-meeting-recorder/main/scripts/install-whisper-windows.ps1 | iex";

export default function DepsTab() {
  const { t } = useTranslation();
  const [deps, setDeps] = useState<DepsCheck | null>(null);

  const recheck = async () => {
    try { setDeps(await invoke<DepsCheck>("deps_check")); } catch {}
  };
  useEffect(() => { recheck(); }, []);

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("deps.title")}</h3>
      <p style={{ fontSize: 12, opacity: 0.7 }}>{t("deps.hint")}</p>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <DepRow label={t("deps.whisper_cli")} ok={deps?.whisper_cli_ok ?? false} path={deps?.whisper_cli_path ?? "—"} t={t} />
        <DepRow label={t("deps.model")} ok={deps?.model_ok ?? false} path={deps?.model_path ?? "—"} t={t} />
      </div>
      <button className="mmr-btn" onClick={recheck} style={{ marginTop: 12 }}>{t("deps.recheck")}</button>

      <h4 style={{ marginTop: 18, marginBottom: 6 }}>{t("deps.linux_install")}</h4>
      <pre style={{ background: "var(--c-surface)", padding: 10, borderRadius: 6, fontSize: 11, overflow: "auto" }}>
        {LINUX_CMD}
      </pre>

      <h4 style={{ marginTop: 14, marginBottom: 6 }}>{t("deps.windows_install")}</h4>
      <pre style={{ background: "var(--c-surface)", padding: 10, borderRadius: 6, fontSize: 11, overflow: "auto" }}>
        {WINDOWS_CMD}
      </pre>
    </div>
  );
}

function DepRow({ label, ok, path, t }: { label: string; ok: boolean; path: string; t: (k: string) => string }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
      <span style={{ minWidth: 140 }}>{label}</span>
      <span style={{ color: ok ? "var(--c-accent)" : "var(--c-danger)" }}>
        {ok ? t("deps.found") : t("deps.missing")}
      </span>
      <code style={{ fontSize: 11, opacity: 0.6, marginLeft: "auto" }}>{path}</code>
    </div>
  );
}
