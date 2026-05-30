import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";

// 把現成音/影檔轉成逐字稿(非 session 錄音)。後端 file_transcribe_one:
// ffmpeg 抽 WAV → whisper-cli → text。deps 走既有 deps_check(已含 ffmpeg_ok)。

type DepsCheck = {
  ffmpeg_ok: boolean;
  whisper_cli_ok: boolean;
  model_ok: boolean;
};
type FileTranscript = {
  source_path: string;
  text: string;
  duration_secs: number;
};

const MEDIA_EXTS = [
  "wav", "mp3", "m4a", "flac", "ogg", "aac", "opus", "wma",
  "mp4", "mkv", "webm", "mov", "avi",
];

export default function FilesTab() {
  const { t } = useTranslation();
  const [deps, setDeps] = useState<DepsCheck | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<FileTranscript | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const recheck = async () => {
    try { setDeps(await invoke<DepsCheck>("deps_check")); } catch {}
  };
  useEffect(() => { recheck(); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const depsOk = !!deps && deps.ffmpeg_ok && deps.whisper_cli_ok && deps.model_ok;

  const pick = async () => {
    setErr(null); setResult(null); setSavedAt(null);
    const sel = await open({ multiple: false, filters: [{ name: "Media", extensions: MEDIA_EXTS }] });
    if (typeof sel === "string") setPath(sel);
  };

  const transcribe = async () => {
    if (!path) return;
    setBusy(true); setErr(null); setResult(null); setSavedAt(null);
    try {
      setResult(await invoke<FileTranscript>("file_transcribe_one", { path }));
    } catch (e: any) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const copy = async () => {
    if (!result) return;
    try {
      await navigator.clipboard.writeText(result.text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch { /* 使用者可自行從 textarea 選取複製 */ }
  };

  const saveTxt = async () => {
    if (!result) return;
    try {
      const out = await invoke<string>("file_transcribe_save_txt", {
        sourcePath: result.source_path,
        text: result.text,
      });
      setSavedAt(out);
    } catch (e: any) {
      setErr(String(e));
    }
  };

  const fileName = path ? path.split(/[\\/]/).pop() : null;

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("files.title")}</h3>
      <p style={{ fontSize: 11, color: "var(--text-dim)", marginBottom: 12 }}>{t("files.hint")}</p>

      <div style={{ display: "flex", flexDirection: "column" }}>
        <DepRow label={t("files.dep_ffmpeg")} ok={deps?.ffmpeg_ok ?? false} t={t} />
        <DepRow label={t("files.dep_whisper")} ok={deps?.whisper_cli_ok ?? false} t={t} />
        <DepRow label={t("files.dep_model")} ok={deps?.model_ok ?? false} t={t} />
      </div>
      {deps && !depsOk && (
        <p style={{ fontSize: 10.5, color: "var(--text-dim)", margin: "6px 0" }}>{t("files.deps_missing")}</p>
      )}

      <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 12 }}>
        <button className="mmr-btn" onClick={pick} disabled={busy}>{t("files.pick")}</button>
        <button className="mmr-btn primary" onClick={transcribe} disabled={!path || !depsOk || busy}>
          {busy
            ? (<><span className="spinner-rotate" style={{ marginRight: 6 }}>↻</span>{t("files.transcribing")}</>)
            : t("files.start")}
        </button>
      </div>
      {fileName && <p style={{ fontSize: 11, color: "var(--text-secondary)", margin: "8px 0 0" }}>{fileName}</p>}

      {err && (
        <div className="callout" style={{ marginTop: 10, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>
          ⚠ {err}
        </div>
      )}

      {result && (
        <div style={{ marginTop: 12 }}>
          <textarea
            readOnly
            value={result.text}
            style={{ width: "100%", minHeight: 180, fontSize: 12, fontFamily: "inherit", resize: "vertical", boxSizing: "border-box" }}
          />
          <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 8 }}>
            <button className="mmr-btn" onClick={copy}>{copied ? t("files.copied") : t("files.copy")}</button>
            <button className="mmr-btn" onClick={saveTxt}>{t("files.save_txt")}</button>
            {savedAt && <span style={{ fontSize: 10.5, color: "var(--found-color)" }}>{t("files.saved", { path: savedAt })}</span>}
          </div>
        </div>
      )}
    </div>
  );
}

function DepRow({ label, ok, t }: { label: string; ok: boolean; t: (k: string) => string }) {
  return (
    <div className="dep-row">
      <span className="label">{label}</span>
      <span className={ok ? "ok" : "miss"}>{ok ? t("deps.found") : t("deps.missing")}</span>
    </div>
  );
}
