// src/tabs/PeopleTab.tsx
//
// 聲紋人員分頁:註冊聲紋、列出已存人員、補錄/改名/刪除。
// Tauri commands 走 camelCase 參數(Tauri v2 auto-transform)。
// 返回值欄位原樣(sample_count snake_case — backend returns as-named, no transform)。

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

interface VoiceprintInfo {
  id: string;
  name: string;
  sample_count: number; // snake_case — backend as-named
}

export default function PeopleTab() {
  const { t } = useTranslation();

  const [modelsPresent, setModelsPresent] = useState<boolean | null>(null);
  const [list, setList] = useState<VoiceprintInfo[]>([]);
  const [listErr, setListErr] = useState<string | null>(null);

  // Enroll state
  const [enrollName, setEnrollName] = useState("");
  const [recording, setRecording] = useState(false);
  const [enrollErr, setEnrollErr] = useState<string | null>(null);
  const [enrollPending, setEnrollPending] = useState(false);

  // Timer for recording duration display
  const [elapsed, setElapsed] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Re-enroll state: which person is being supplemented (null = fresh enroll)
  const [reenrollId, setReenrollId] = useState<string | null>(null);

  // Rename state: which person is being renamed inline
  const [renameId, setRenameId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameErr, setRenameErr] = useState<string | null>(null);

  // Delete errors
  const [deleteErr, setDeleteErr] = useState<string | null>(null);

  const checkModels = async () => {
    try {
      const present = await invoke<boolean>("voiceprint_models_present");
      setModelsPresent(present);
    } catch {
      setModelsPresent(false);
    }
  };

  const reloadList = async () => {
    setListErr(null);
    try {
      const rows = await invoke<VoiceprintInfo[]>("list_voiceprints");
      setList(rows);
    } catch (e: unknown) {
      setListErr(String(e));
    }
  };

  useEffect(() => {
    checkModels();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (modelsPresent) reloadList();
  }, [modelsPresent]); // eslint-disable-line react-hooks/exhaustive-deps

  const startTimer = () => {
    setElapsed(0);
    timerRef.current = setInterval(() => setElapsed((s) => s + 1), 1000);
  };

  const stopTimer = () => {
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  };

  const fmtElapsed = (s: number) => {
    const m = Math.floor(s / 60);
    const sec = s % 60;
    return `${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
  };

  // Start a fresh enroll recording
  const handleStartRecording = async () => {
    setEnrollErr(null);
    try {
      await invoke("enroll_voice_start");
      setRecording(true);
      setReenrollId(null);
      startTimer();
    } catch (e: unknown) {
      setEnrollErr(String(e));
    }
  };

  // Start a re-enroll (supplement) recording for an existing person
  const handleStartReenroll = async (person: VoiceprintInfo) => {
    setEnrollErr(null);
    setEnrollName(person.name);
    try {
      await invoke("enroll_voice_start");
      setRecording(true);
      setReenrollId(person.id);
      startTimer();
    } catch (e: unknown) {
      setEnrollErr(String(e));
    }
  };

  // Finish recording and save (accumulates if name already exists)
  const handleFinish = async () => {
    if (!enrollName.trim()) return;
    setEnrollPending(true);
    setEnrollErr(null);
    stopTimer();
    try {
      await invoke("enroll_voice_finish", { name: enrollName.trim() });
      setRecording(false);
      const wasReenroll = reenrollId !== null;
      setReenrollId(null);
      if (!wasReenroll) setEnrollName("");
      await reloadList();
    } catch (e: unknown) {
      setEnrollErr(String(e));
      // Keep recording state so user can see the error in context
    } finally {
      setEnrollPending(false);
    }
  };

  // Abort — stop mic and discard the temp WAV without embedding or touching the registry.
  const handleCancelRecording = async () => {
    stopTimer();
    const wasReenroll = reenrollId !== null;
    try {
      await invoke("enroll_voice_cancel");
    } catch {
      // best-effort — mic may already be stopped
    }
    setRecording(false);
    setReenrollId(null);
    setEnrollErr(null);
    setElapsed(0);
    if (wasReenroll) setEnrollName("");
    await reloadList();
  };

  // Rename
  const handleRenameStart = (person: VoiceprintInfo) => {
    setRenameId(person.id);
    setRenameDraft(person.name);
    setRenameErr(null);
  };

  const handleRenameCommit = async (id: string) => {
    if (!renameDraft.trim()) return;
    setRenameErr(null);
    try {
      await invoke("rename_voiceprint", { id, name: renameDraft.trim() });
      setRenameId(null);
      await reloadList();
    } catch (e: unknown) {
      setRenameErr(String(e));
    }
  };

  const handleRenameCancel = () => {
    setRenameId(null);
    setRenameDraft("");
    setRenameErr(null);
  };

  // Delete
  const handleDelete = async (id: string) => {
    setDeleteErr(null);
    try {
      await invoke("remove_voiceprint", { id });
      await reloadList();
    } catch (e: unknown) {
      setDeleteErr(String(e));
    }
  };

  // --- Render ---

  if (modelsPresent === null) {
    return (
      <div>
        <h3 style={{ marginTop: 0 }}>{t("people.title")}</h3>
        <p style={{ color: "var(--text-dim)", fontSize: 11 }}>{t("people.checking")}</p>
      </div>
    );
  }

  if (!modelsPresent) {
    return (
      <div>
        <h3 style={{ marginTop: 0 }}>{t("people.title")}</h3>
        <div className="callout" style={{ marginTop: 12 }}>
          {t("people.no_model_hint")}
        </div>
      </div>
    );
  }

  return (
    <div>
      <h3 style={{ marginTop: 0 }}>{t("people.title")}</h3>
      <p style={{ fontSize: 11, color: "var(--text-dim)", marginBottom: 12 }}>
        {t("people.auto_name_hint")}
      </p>

      {/* Enroll section */}
      <h4 style={{ marginBottom: 8, color: "var(--text-secondary)", fontSize: 12 }}>
        {t("people.enroll_title")}
      </h4>

      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <input
          className="mi-input"
          type="text"
          placeholder={t("people.name_placeholder")}
          value={enrollName}
          onChange={(e) => setEnrollName(e.target.value)}
          disabled={recording}
          style={{ flex: 1, minWidth: 120 }}
        />
        {!recording ? (
          <button
            className="mmr-btn primary"
            onClick={handleStartRecording}
            disabled={!enrollName.trim()}
          >
            {t("people.start_record")}
          </button>
        ) : (
          <>
            <button
              className="mmr-btn primary"
              onClick={handleFinish}
              disabled={!enrollName.trim() || enrollPending}
            >
              {enrollPending ? t("people.saving") : t("people.finish")}
            </button>
            <button
              className="mmr-btn"
              onClick={handleCancelRecording}
              disabled={enrollPending}
            >
              {t("people.cancel")}
            </button>
          </>
        )}
      </div>

      {recording && (
        <div style={{ marginTop: 10, display: "flex", flexDirection: "column", gap: 4 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span className="capsule-dot recording" />
            <span style={{ fontSize: 11, color: "var(--text-secondary)", fontVariantNumeric: "tabular-nums" }}>
              {t("people.recording_label")} {fmtElapsed(elapsed)}
            </span>
          </div>
          <p style={{ fontSize: 11, color: "var(--text-dim)", margin: 0 }}>
            {t("people.speak_hint")}
          </p>
        </div>
      )}

      {enrollErr && (
        <div className="callout" style={{ marginTop: 8, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>
          {enrollErr}
        </div>
      )}

      {/* People list */}
      <h4 style={{ marginTop: 20, marginBottom: 8, color: "var(--text-secondary)", fontSize: 12 }}>
        {t("people.list_title")}
      </h4>

      {listErr && (
        <div className="callout" style={{ marginBottom: 8, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>
          {listErr}
        </div>
      )}

      {deleteErr && (
        <div className="callout" style={{ marginBottom: 8, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>
          {deleteErr}
        </div>
      )}

      {list.length === 0 ? (
        <p style={{ fontSize: 11, color: "var(--text-dim)", margin: 0 }}>{t("people.list_empty")}</p>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {list.map((person) => (
            <div
              key={person.id}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "6px 10px",
                borderRadius: 8,
                background: "var(--btn-bg)",
                border: "0.5px solid var(--border)",
                flexWrap: "wrap",
              }}
            >
              {renameId === person.id ? (
                <>
                  <input
                    className="mi-input"
                    type="text"
                    value={renameDraft}
                    onChange={(e) => setRenameDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleRenameCommit(person.id);
                      if (e.key === "Escape") handleRenameCancel();
                    }}
                    style={{ flex: 1, minWidth: 80 }}
                    autoFocus
                  />
                  <button
                    className="mmr-btn primary"
                    onClick={() => handleRenameCommit(person.id)}
                    disabled={!renameDraft.trim()}
                  >
                    {t("people.rename_save")}
                  </button>
                  <button className="mmr-btn" onClick={handleRenameCancel}>
                    {t("people.rename_cancel")}
                  </button>
                  {renameErr && (
                    <span style={{ fontSize: 10.5, color: "var(--danger-color)" }}>{renameErr}</span>
                  )}
                </>
              ) : (
                <>
                  <span style={{ flex: 1, fontSize: 13, color: "var(--text)", fontWeight: 500 }}>
                    {person.name}
                  </span>
                  <span
                    style={{
                      fontSize: 10.5,
                      padding: "1px 6px",
                      borderRadius: 999,
                      background: "var(--pill-bg)",
                      color: "var(--text-dim)",
                    }}
                  >
                    {t("people.sample_count", { count: person.sample_count })}
                  </span>
                  <button
                    className="mmr-btn"
                    onClick={() => handleStartReenroll(person)}
                    disabled={recording}
                    title={t("people.reenroll_title")}
                  >
                    {t("people.reenroll")}
                  </button>
                  <button
                    className="mmr-btn"
                    onClick={() => handleRenameStart(person)}
                    disabled={recording}
                    title={t("people.rename_title")}
                  >
                    {t("people.rename")}
                  </button>
                  <button
                    className="mmr-btn danger"
                    onClick={() => handleDelete(person.id)}
                    disabled={recording}
                    title={t("people.delete_title")}
                  >
                    {t("people.delete")}
                  </button>
                </>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
