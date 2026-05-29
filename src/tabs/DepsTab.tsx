import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import CopyCodeBlock from "../components/CopyCodeBlock";

type DepsCheck = {
  whisper_cli_ok: boolean;
  whisper_cli_path: string;
  model_ok: boolean;
  model_path: string;
};
type GpuStatus = { gpu_name: string | null; cuda_toolkit: boolean; whisper_gpu_build: boolean };

const LINUX_CMD = "curl -fsSL https://raw.githubusercontent.com/yazelin/mori-meeting-recorder/main/scripts/install-whisper-linux.sh | bash";
const WINDOWS_CMD = "iwr https://raw.githubusercontent.com/yazelin/mori-meeting-recorder/main/scripts/install-whisper-windows.ps1 | iex";
const CUDA_INSTALL_CMD = "sudo apt install -y nvidia-cuda-toolkit";
// 強制重建(script 看到 whisper-cli 已存在會跳過 → 先刪掉才會用 CUDA 重編)
const GPU_REBUILD_CMD = "rm -f ~/.mori/bin/whisper-cli && " + LINUX_CMD;

export default function DepsTab() {
  const { t } = useTranslation();
  const [deps, setDeps] = useState<DepsCheck | null>(null);
  const [dl, setDl] = useState<{ active: boolean; downloaded: number; total: number }>({ active: false, downloaded: 0, total: 0 });
  const [dlErr, setDlErr] = useState<string | null>(null);

  const [gpu, setGpu] = useState<GpuStatus | null>(null);

  const recheck = async () => {
    try { setDeps(await invoke<DepsCheck>("deps_check")); } catch {}
    try { setGpu(await invoke<GpuStatus>("gpu_status")); } catch {}
  };
  useEffect(() => { recheck(); }, []);

  // app 內下載目前選的模型 + polling 進度 bar。
  const downloadModel = async () => {
    setDlErr(null);
    setDl({ active: true, downloaded: 0, total: 0 });
    const id = setInterval(async () => {
      try { setDl(await invoke("download_progress")); } catch {}
    }, 500);
    try {
      await invoke("download_model");
      await recheck();
    } catch (e: any) {
      setDlErr(String(e));
    } finally {
      clearInterval(id);
      setDl({ active: false, downloaded: 0, total: 0 });
    }
  };
  const mb = (b: number) => (b / 1_000_000).toFixed(0);
  const pct = dl.total > 0 ? Math.round((dl.downloaded / dl.total) * 100) : 0;

  // 目前選的模型(從 model_path 取檔名)→ 給對應的下載指令。換成 large-v3-turbo 時這裡就會
  // 變成 turbo 的下載指令(全安裝 script 只抓 small,turbo 要另外下載)。
  const modelFile = (deps?.model_path ?? "").split(/[\\/]/).pop() || "ggml-small.bin";
  const modelDlCmd = `curl -L --fail -o "${deps?.model_path ?? `~/.mori/models/${modelFile}`}" "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${modelFile}"`;

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("deps.title")}</h3>
      <p style={{ fontSize: 11, color: "var(--text-dim)", marginBottom: 12 }}>{t("deps.hint")}</p>

      <div style={{ display: "flex", flexDirection: "column" }}>
        <DepRow label={t("deps.whisper_cli")} ok={deps?.whisper_cli_ok ?? false} path={deps?.whisper_cli_path ?? "—"} t={t} />
        <DepRow label={t("deps.model")} ok={deps?.model_ok ?? false} path={deps?.model_path ?? "—"} t={t} />
      </div>
      <button className="mmr-btn" onClick={recheck} style={{ marginTop: 12 }}>{t("deps.recheck")}</button>

      <h4>{t("deps.gpu_title")}</h4>
      <div className="dep-row">
        <span className="label">{t("deps.gpu_accel")}</span>
        <span className={gpu?.whisper_gpu_build ? "ok" : "miss"}>
          {gpu?.whisper_gpu_build ? t("deps.gpu_on") : t("deps.gpu_off")}
        </span>
        <code>{gpu?.gpu_name ?? t("deps.gpu_none")}</code>
      </div>
      {gpu && gpu.gpu_name && !gpu.whisper_gpu_build && (
        <>
          <p style={{ fontSize: 10.5, color: "var(--text-dim)", margin: "6px 0" }}>
            {gpu.cuda_toolkit ? t("deps.gpu_steps_rebuild") : t("deps.gpu_steps_cuda")}
          </p>
          {!gpu.cuda_toolkit && <CopyCodeBlock code={CUDA_INSTALL_CMD} />}
          <CopyCodeBlock code={GPU_REBUILD_CMD} />
        </>
      )}
      {gpu && !gpu.gpu_name && (
        <p style={{ fontSize: 10.5, color: "var(--text-dim)", margin: "6px 0" }}>{t("deps.gpu_no_hint")}</p>
      )}

      <h4>{t("deps.download_model")}</h4>
      <p style={{ fontSize: 10.5, color: "var(--text-dim)", margin: "0 0 8px" }}>{t("deps.download_model_hint", { file: modelFile })}</p>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <button className="mmr-btn primary" onClick={downloadModel} disabled={dl.active}>
          {dl.active ? t("deps.downloading") : t("deps.download_btn")}
        </button>
        {dl.active && (
          <span style={{ fontSize: 11, color: "var(--text-secondary)", fontVariantNumeric: "tabular-nums" }}>
            {dl.total > 0 ? `${pct}% · ${mb(dl.downloaded)}/${mb(dl.total)} MB` : t("deps.downloading")}
          </span>
        )}
      </div>
      {dl.active && (
        <div className="dl-bar"><div className="dl-bar-fill" style={{ width: dl.total > 0 ? `${pct}%` : "40%" }} /></div>
      )}
      {dlErr && <div className="callout" style={{ marginTop: 8, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>⚠ {dlErr}</div>}
      <details style={{ marginTop: 8 }}>
        <summary style={{ fontSize: 10.5, color: "var(--text-dim)", cursor: "pointer" }}>{t("deps.or_terminal")}</summary>
        <CopyCodeBlock code={modelDlCmd} />
      </details>

      <h4>{t("deps.linux_install")}</h4>
      <CopyCodeBlock code={LINUX_CMD} />

      <h4>{t("deps.windows_install")}</h4>
      <CopyCodeBlock code={WINDOWS_CMD} />
    </div>
  );
}

function DepRow({ label, ok, path, t }: { label: string; ok: boolean; path: string; t: (k: string) => string }) {
  return (
    <div className="dep-row">
      <span className="label">{label}</span>
      <span className={ok ? "ok" : "miss"}>
        {ok ? t("deps.found") : t("deps.missing")}
      </span>
      <code>{path}</code>
    </div>
  );
}
