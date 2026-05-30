// src/tabs/SessionWorkspace.tsx
//
// 會議工作區:讀取一場 session 的 meeting-info / speakers / transcript，
// 提供主題/人員編輯、講者分離、講者改名、重新匯出等功能。
// Tauri commands 全走 camelCase 參數(Tauri v2 auto-transform)。

import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import Select, { SelectOption } from "../components/Select";

interface Segment {
  id: string;
  session_id: string;
  track: string;
  source_kind: string;
  visibility: string;
  start_ms: number;
  end_ms: number;
  text: string;
  is_final: boolean;
  confidence: number | null;
  speaker: string | null;
  speaker_mixed: boolean;
  supplement: boolean;
}

interface SpeakerInfo {
  id: string;
  display: string;
  track: string;
}

interface MeetingInfo {
  topic: string;
  participants: string;
}

// 後端 SummaryResult(serde snake_case;Tauri 回傳值不 camelCase 化)。
interface SummaryResult {
  public_backend: string; // "groq" | "ollama" | "none" | "(failed)"
  internal_backend: string;
  public_chars: number;
  internal_chars: number;
  redaction_count: number;
}

// get_config 帶出的摘要相關欄位(只用得到 force-local 預設)。
interface SummaryConfig {
  summary_force_local_default: boolean;
}

interface Props {
  sessionId: string;
  onBack: () => void;
}

function fmtMs(ms: number): string {
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function splitParticipants(raw: string): string[] {
  return raw
    .split(/[,;，；\n\/]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

export default function SessionWorkspace({ sessionId, onBack }: Props) {
  const { t } = useTranslation();

  // Data state
  const [info, setInfo] = useState<MeetingInfo>({ topic: "", participants: "" });
  const [speakers, setSpeakers] = useState<SpeakerInfo[]>([]);
  const [segments, setSegments] = useState<Segment[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadErr, setLoadErr] = useState<string | null>(null);

  // UI state
  const [topic, setTopic] = useState("");
  const [participants, setParticipants] = useState("");
  const [infoSaved, setInfoSaved] = useState(false);
  const [infoErr, setInfoErr] = useState<string | null>(null);

  const [diarizing, setDiarizing] = useState(false);
  const [diarErr, setDiarErr] = useState<string | null>(null);
  const [diarHint, setDiarHint] = useState<string | null>(null);

  const [reexporting, setReexporting] = useState(false);
  const [reexportMsg, setReexportMsg] = useState<string | null>(null);
  const [reexportErr, setReexportErr] = useState<string | null>(null);

  // Summary state — 獨立 state,不與 reexport 共用(§8.2 gotcha:同時觸發會互相 stomp)。
  const [summarizing, setSummarizing] = useState(false);
  const [summaryMsg, setSummaryMsg] = useState<string | null>(null);
  const [summaryErr, setSummaryErr] = useState<string | null>(null);
  const [forceLocal, setForceLocal] = useState(false);
  const [summaryPublic, setSummaryPublic] = useState<string | null>(null);
  const [summaryInternal, setSummaryInternal] = useState<string | null>(null);
  const [publicBackend, setPublicBackend] = useState<string | null>(null);
  const [internalBackend, setInternalBackend] = useState<string | null>(null);
  const [summaryTab, setSummaryTab] = useState<"public" | "internal">("public");

  // Speaker rename pending map: id -> display value being edited
  const [speakerEdits, setSpeakerEdits] = useState<Record<string, string>>({});
  const [speakerSaving, setSpeakerSaving] = useState<Record<string, boolean>>({});

  // Speaker merge selection
  const [selectedSpeakers, setSelectedSpeakers] = useState<string[]>([]);
  const [merging, setMerging] = useState(false);
  const [mergeErr, setMergeErr] = useState<string | null>(null);

  // Per-segment speaker reassign loading
  const [segSaving, setSegSaving] = useState<Record<string, boolean>>({});

  // Per-segment text edit state
  const [segTextEditing, setSegTextEditing] = useState<Record<string, string | null>>({});
  const [segTextSaving, setSegTextSaving] = useState<Record<string, boolean>>({});

  // Per-segment supplement toggle saving
  const [segSupplementSaving, setSegSupplementSaving] = useState<Record<string, boolean>>({});

  const loadAll = useCallback(async () => {
    setLoadErr(null);
    try {
      const [mi, spk, segs] = await Promise.all([
        invoke<MeetingInfo>("read_meeting_info", { sessionId }),
        invoke<SpeakerInfo[]>("read_speakers_cmd", { sessionId }),
        invoke<Segment[]>("read_session_transcript", { sessionId }),
      ]);
      setInfo(mi);
      setTopic(mi.topic);
      setParticipants(mi.participants);
      setSpeakers(spk);
      // Reset edits to match current speaker display names
      const edits: Record<string, string> = {};
      spk.forEach((s) => { edits[s.id] = s.display; });
      setSpeakerEdits(edits);
      setSegments(segs);
    } catch (e: any) {
      console.error("SessionWorkspace loadAll:", e);
      setLoadErr(String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  // Reload only speakers + transcript (used after diarize / merge / rename)
  const reloadSpeakersAndTranscript = useCallback(async () => {
    const [spk, segs] = await Promise.all([
      invoke<SpeakerInfo[]>("read_speakers_cmd", { sessionId }),
      invoke<Segment[]>("read_session_transcript", { sessionId }),
    ]);
    setSpeakers(spk);
    const edits: Record<string, string> = {};
    spk.forEach((s) => { edits[s.id] = s.display; });
    setSpeakerEdits(edits);
    setSegments(segs);
  }, [sessionId]);

  // Reload only transcript (used after per-segment reassign)
  const reloadTranscript = useCallback(async () => {
    const segs = await invoke<Segment[]>("read_session_transcript", { sessionId });
    setSegments(segs);
  }, [sessionId]);

  useEffect(() => {
    setLoading(true);
    loadAll();
  }, [loadAll]);

  const saveInfo = async () => {
    setInfoErr(null);
    setInfoSaved(false);
    try {
      await invoke("set_meeting_info_for", { sessionId, topic, participants });
      setInfo({ topic, participants });
      setInfoSaved(true);
      setTimeout(() => setInfoSaved(false), 2000);
    } catch (e: any) {
      setInfoErr(String(e));
    }
  };

  const runDiarize = async () => {
    // Confirm if already diarized
    if (speakers.length > 0) {
      const ok = window.confirm(t("workspace.diar_rerun_confirm"));
      if (!ok) return;
    }
    setDiarErr(null);
    setDiarHint(null);
    // Check model presence first
    let present = false;
    try { present = await invoke<boolean>("diar_models_present"); } catch {}
    if (!present) {
      setDiarHint(t("workspace.diar_no_model_hint"));
      return;
    }
    setDiarizing(true);
    try {
      await invoke("diarize_session", { sessionId });
      // Reload speakers + transcript after diarization
      await reloadSpeakersAndTranscript();
      setSelectedSpeakers([]);
    } catch (e: any) {
      setDiarErr(String(e));
    } finally {
      setDiarizing(false);
    }
  };

  const renameSpeaker = async (id: string, display: string) => {
    if (speakerSaving[id]) return;
    setSpeakerSaving((prev) => ({ ...prev, [id]: true }));
    try {
      await invoke("rename_speaker_cmd", { sessionId, id, display });
      const spk = await invoke<SpeakerInfo[]>("read_speakers_cmd", { sessionId });
      setSpeakers(spk);
      const edits: Record<string, string> = {};
      spk.forEach((s) => { edits[s.id] = s.display; });
      setSpeakerEdits(edits);
    } catch (e: any) {
      console.error("rename_speaker_cmd:", e);
    } finally {
      setSpeakerSaving((prev) => ({ ...prev, [id]: false }));
    }
  };

  const mergeSpeakers = async () => {
    if (selectedSpeakers.length < 2) return;
    setMergeErr(null);
    setMerging(true);
    try {
      // keepId = first selected in speakers-list order; mergeIds = the rest
      const orderedSelected = speakers
        .map((s) => s.id)
        .filter((id) => selectedSpeakers.includes(id));
      const keepId = orderedSelected[0];
      const mergeIds = orderedSelected.slice(1);
      await invoke("merge_speakers", { sessionId, keepId, mergeIds });
      setSelectedSpeakers([]);
      await reloadSpeakersAndTranscript();
    } catch (e: any) {
      setMergeErr(String(e));
    } finally {
      setMerging(false);
    }
  };

  const toggleSpeakerSelection = (id: string) => {
    setSelectedSpeakers((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]
    );
  };

  const setSegmentSpeaker = async (seg: Segment, speakerId: string) => {
    // Key by (track, start_ms) — seg.id is NOT unique per track (VAD resets per clip).
    const key = `${seg.track}-${seg.start_ms}`;
    setSegSaving((prev) => ({ ...prev, [key]: true }));
    try {
      await invoke("set_segment_speaker", {
        sessionId,
        track: seg.track,
        startMs: seg.start_ms,
        speakerId,
      });
      await reloadTranscript();
    } catch (e: any) {
      console.error("set_segment_speaker:", e);
    } finally {
      setSegSaving((prev) => ({ ...prev, [key]: false }));
    }
  };

  const saveSegmentText = async (seg: Segment, text: string) => {
    const key = `${seg.track}-${seg.start_ms}`;
    // No change — just close editor
    if (text === seg.text) {
      setSegTextEditing((prev) => ({ ...prev, [key]: null }));
      return;
    }
    setSegTextSaving((prev) => ({ ...prev, [key]: true }));
    try {
      await invoke("set_segment_text", {
        sessionId,
        track: seg.track,
        startMs: seg.start_ms,
        text,
      });
      await reloadTranscript();
    } catch (e: any) {
      console.error("set_segment_text:", e);
    } finally {
      setSegTextSaving((prev) => ({ ...prev, [key]: false }));
      setSegTextEditing((prev) => ({ ...prev, [key]: null }));
    }
  };

  const toggleSegmentSupplement = async (seg: Segment) => {
    const key = `${seg.track}-${seg.start_ms}`;
    setSegSupplementSaving((prev) => ({ ...prev, [key]: true }));
    try {
      await invoke("set_segment_supplement", {
        sessionId,
        track: seg.track,
        startMs: seg.start_ms,
        supplement: !seg.supplement,
      });
      await reloadTranscript();
    } catch (e: any) {
      console.error("set_segment_supplement:", e);
    } finally {
      setSegSupplementSaving((prev) => ({ ...prev, [key]: false }));
    }
  };

  const reexport = async () => {
    setReexportErr(null);
    setReexportMsg(null);
    setReexporting(true);
    try {
      await invoke("reexport_session", { sessionId });
      setReexportMsg(t("workspace.reexport_ok"));
      setTimeout(() => setReexportMsg(null), 3000);
    } catch (e: any) {
      setReexportErr(String(e));
    } finally {
      setReexporting(false);
    }
  };

  // 讀已生成的兩份摘要 .md(缺檔回 null）。
  const reloadSummaries = useCallback(async () => {
    const [pub, int] = await Promise.all([
      invoke<string | null>("read_summary_md", { sessionId, kind: "public" }),
      invoke<string | null>("read_summary_md", { sessionId, kind: "internal" }),
    ]);
    setSummaryPublic(pub ?? null);
    setSummaryInternal(int ?? null);
  }, [sessionId]);

  // 進工作區:讀 force-local 預設 + 既有摘要(各自獨立,失敗不互相影響)。
  useEffect(() => {
    invoke<SummaryConfig>("get_config")
      .then((cfg) => setForceLocal(!!cfg.summary_force_local_default))
      .catch(() => {});
    reloadSummaries().catch((e) => console.error("reloadSummaries:", e));
  }, [reloadSummaries]);

  // handler 鏡射 reexport():invoke summarize_session → set 兩個 backend → reload 兩份 .md。
  const summarize = async () => {
    if (summarizing) return;
    setSummaryErr(null);
    setSummaryMsg(null);
    setSummarizing(true);
    try {
      const result = await invoke<SummaryResult>("summarize_session", {
        sessionId,
        forceLocal,
      });
      setPublicBackend(result.public_backend);
      setInternalBackend(result.internal_backend);
      await reloadSummaries();
      setSummaryMsg(
        result.redaction_count > 0
          ? t("workspace.summary_ok_redacted", { count: result.redaction_count })
          : t("workspace.summary_ok")
      );
      setTimeout(() => setSummaryMsg(null), 4000);
    } catch (e: any) {
      // 部分 / 全失敗時後端回 Err 字串,直接顯示;已成功那遍的 .md 仍重載。
      setSummaryErr(String(e));
      await reloadSummaries().catch(() => {});
    } finally {
      setSummarizing(false);
    }
  };

  // 內部補充即時預覽:直接讀記憶體 segments(supplement=true 的麥克風段),不必先重匯出。
  const supplementSegments = segments.filter(
    (s) => s.supplement && s.track !== "system"
  );

  // 後端徽章:☁ Groq(雲端)/ ⚡ 本機 Ollama / 其他狀態。
  const renderBackendBadge = (backend: string | null) => {
    if (backend === "groq") {
      return (
        <span className="backend-badge cloud">
          {`☁ ${t("workspace.backend_groq")}`}
        </span>
      );
    }
    if (backend === "ollama") {
      return (
        <span className="backend-badge local">
          {`⚡ ${t("workspace.backend_ollama")}`}
        </span>
      );
    }
    if (backend === "(failed)") {
      return (
        <span className="backend-badge failed">{t("workspace.backend_failed")}</span>
      );
    }
    // "none"(空逐字稿)或 null(尚未生成)
    return <span className="backend-badge none">{t("workspace.backend_none")}</span>;
  };

  // Build speaker display map for transcript rendering
  const speakerDisplay: Record<string, string> = {};
  speakers.forEach((s) => { speakerDisplay[s.id] = s.display; });

  // Speaker options for per-segment Select
  const speakerOptions: SelectOption[] = speakers.map((s) => ({
    value: s.id,
    label: s.display,
  }));

  // Participant name suggestions from current participants field
  const participantOptions: SelectOption[] = [
    ...splitParticipants(participants).map((p) => ({ value: p, label: p })),
  ];

  if (loading) {
    return (
      <div style={{ color: "var(--text-dim)", padding: "20px 0" }}>
        {t("workspace.loading")}
      </div>
    );
  }

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", minHeight: 0 }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: loadErr ? 4 : 14 }}>
        <button className="mmr-btn" onClick={onBack} style={{ flexShrink: 0 }}>
          {t("workspace.back")}
        </button>
        <code style={{ fontSize: 11, color: "var(--text-dim)", fontFamily: "ui-monospace, monospace", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {sessionId}
        </code>
      </div>
      {loadErr && (
        <div style={{ fontSize: 11, color: "var(--danger-color)", marginBottom: 14 }}>{loadErr}</div>
      )}

      {/* Meeting info section */}
      <h4>{t("workspace.info_title")}</h4>
      <div className="meeting-info">
        <div className="mi-field">
          <label className="mi-label">{t("workspace.topic_label")}</label>
          <input
            className="mi-input"
            value={topic}
            onChange={(e) => setTopic(e.target.value)}
            placeholder={t("workspace.topic_ph")}
          />
        </div>
        <div className="mi-field">
          <label className="mi-label">{t("workspace.participants_label")}</label>
          <textarea
            className="mi-input mi-textarea"
            value={participants}
            onChange={(e) => setParticipants(e.target.value)}
            placeholder={t("workspace.participants_ph")}
          />
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <button className="mmr-btn primary" onClick={saveInfo}>
            {t("workspace.save")}
          </button>
          {infoSaved && <span style={{ fontSize: 11, color: "var(--found-color)" }}>{t("workspace.saved")}</span>}
          {infoErr && <span style={{ fontSize: 11, color: "var(--danger-color)" }}>{infoErr}</span>}
        </div>
      </div>

      {/* Diarization section */}
      <h4>{t("workspace.diar_title")}</h4>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <button className="mmr-btn primary" onClick={runDiarize} disabled={diarizing}>
            {diarizing
              ? <><span className="spinner-rotate" style={{ marginRight: 6 }}>↻</span>{t("workspace.diarizing")}</>
              : t("workspace.diar_btn")}
          </button>
          {diarizing && (
            <span style={{ fontSize: 11, color: "var(--text-dim)" }}>{t("workspace.diar_wait")}</span>
          )}
        </div>
        {diarHint && (
          <div className="callout">{diarHint}</div>
        )}
        {diarErr && (
          <div className="callout" style={{ color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>
            {diarErr}
          </div>
        )}
      </div>

      {/* Speaker list */}
      {speakers.length > 0 && (
        <>
          <h4>{t("workspace.speakers_title")}</h4>
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {/* Merge action bar */}
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
              <button
                className="mmr-btn"
                onClick={mergeSpeakers}
                disabled={selectedSpeakers.length < 2 || merging}
              >
                {merging
                  ? <><span className="spinner-rotate" style={{ marginRight: 6 }}>↻</span>{t("workspace.merging")}</>
                  : t("workspace.merge_btn")}
              </button>
              {selectedSpeakers.length >= 2 && (
                <span style={{ fontSize: 11, color: "var(--text-dim)" }}>
                  {t("workspace.merge_hint", { count: selectedSpeakers.length })}
                </span>
              )}
              {mergeErr && (
                <span style={{ fontSize: 11, color: "var(--danger-color)" }}>{mergeErr}</span>
              )}
            </div>

            {speakers.map((spk) => {
              const editVal = speakerEdits[spk.id] ?? spk.display;
              const saving = speakerSaving[spk.id] ?? false;
              const hasParticipants = splitParticipants(participants).length > 0;
              const isSelected = selectedSpeakers.includes(spk.id);
              return (
                <div key={spk.id} style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  {/* Checkbox for merge selection */}
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onChange={() => toggleSpeakerSelection(spk.id)}
                    style={{ accentColor: "var(--accent-color)", flexShrink: 0, cursor: "pointer" }}
                  />
                  <code style={{ fontSize: 10.5, color: "var(--text-dim)", minWidth: 72, fontFamily: "ui-monospace, monospace" }}>
                    {spk.id}
                  </code>
                  {hasParticipants ? (
                    <Select
                      value={editVal}
                      options={[
                        ...participantOptions,
                        { value: editVal, label: editVal },
                      ].filter((o, i, arr) => arr.findIndex((x) => x.value === o.value) === i)}
                      onChange={(v) => {
                        setSpeakerEdits((prev) => ({ ...prev, [spk.id]: v }));
                        renameSpeaker(spk.id, v);
                      }}
                    />
                  ) : (
                    <input
                      className="mi-input"
                      style={{ flex: 1, minWidth: 0 }}
                      value={editVal}
                      onChange={(e) =>
                        setSpeakerEdits((prev) => ({ ...prev, [spk.id]: e.target.value }))
                      }
                      onBlur={(e) => renameSpeaker(spk.id, e.target.value)}
                    />
                  )}
                  {saving && (
                    <span className="spinner-rotate" style={{ fontSize: 11, color: "var(--text-dim)" }}>↻</span>
                  )}
                </div>
              );
            })}
          </div>
        </>
      )}

      {/* Transcript section — flex:1 so it fills remaining height as window grows */}
      <h4>{t("workspace.transcript_title")}</h4>
      {segments.length === 0 ? (
        <p style={{ fontSize: 11, color: "var(--text-dim)" }}>{t("workspace.transcript_empty")}</p>
      ) : (
        <div
          style={{
            border: "0.5px solid var(--border)",
            borderRadius: 10,
            background: "rgba(255,255,255,0.02)",
            padding: "8px 10px",
            flex: 1,
            minHeight: 120,
            overflowY: "auto",
          }}
        >
          {segments.map((seg) => {
            // Key by (track, start_ms) — seg.id is NOT unique per track (VAD resets per clip).
            const segKey = `${seg.track}-${seg.start_ms}`;
            const display = seg.speaker ? (speakerDisplay[seg.speaker] ?? seg.speaker) : t("workspace.unknown_speaker");
            const isSpeakerSaving = segSaving[segKey] ?? false;
            const editingText = segTextEditing[segKey] ?? null;
            const isTextEditing = editingText !== null;
            const isTextSaving = segTextSaving[segKey] ?? false;
            const isSupplementSaving = segSupplementSaving[segKey] ?? false;
            return (
              <div key={segKey} style={{ display: "flex", gap: 8, marginBottom: 6, fontSize: 12, lineHeight: 1.5, alignItems: "flex-start" }}>
                <span style={{ fontFamily: "ui-monospace, monospace", fontSize: 10, color: "var(--text-dim)", flexShrink: 0, paddingTop: 3 }}>
                  {fmtMs(seg.start_ms)}
                </span>
                <span
                  className="seg-pill"
                  data-tone={seg.track === "system" ? "public" : "internal"}
                  style={{ flexShrink: 0, alignSelf: "center" }}
                >
                  {seg.track === "system" ? t("workspace.src_system") : t("workspace.src_mic")}
                </span>
                {/* Per-segment speaker Select (only when speakers exist) */}
                {speakerOptions.length > 0 ? (
                  <span style={{ flexShrink: 0, position: "relative" }}>
                    <Select
                      value={seg.speaker ?? ""}
                      options={speakerOptions}
                      onChange={(newId) => setSegmentSpeaker(seg, newId)}
                    />
                    {isSpeakerSaving && (
                      <span className="spinner-rotate" style={{ fontSize: 10, color: "var(--text-dim)", position: "absolute", top: 0, right: -14 }}>↻</span>
                    )}
                  </span>
                ) : (
                  <span style={{ color: "var(--text-secondary)", flexShrink: 0 }}>
                    {display}
                    {seg.speaker_mixed && (
                      <span
                        className="mori-pill-badge"
                        style={{
                          marginLeft: 4,
                          fontSize: 9,
                          padding: "1px 5px",
                          borderRadius: 999,
                          background: "var(--pill-bg)",
                          color: "var(--text-dim)",
                          verticalAlign: "middle",
                        }}
                      >
                        {t("workspace.mixed_badge")}
                      </span>
                    )}
                    {":"}
                  </span>
                )}
                {seg.speaker_mixed && speakerOptions.length > 0 && (
                  <span
                    className="mori-pill-badge"
                    style={{
                      flexShrink: 0,
                      fontSize: 9,
                      padding: "1px 5px",
                      borderRadius: 999,
                      background: "var(--pill-bg)",
                      color: "var(--text-dim)",
                      alignSelf: "center",
                    }}
                  >
                    {t("workspace.mixed_badge")}
                  </span>
                )}
                {/* Feature A: inline text edit */}
                {isTextEditing ? (
                  <span style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 4 }}>
                    <textarea
                      autoFocus
                      disabled={isTextSaving}
                      value={editingText}
                      onChange={(e) =>
                        setSegTextEditing((prev) => ({ ...prev, [segKey]: e.target.value }))
                      }
                      onBlur={(e) => saveSegmentText(seg, e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Escape") {
                          e.preventDefault();
                          setSegTextEditing((prev) => ({ ...prev, [segKey]: null }));
                        } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                          e.preventDefault();
                          saveSegmentText(seg, editingText);
                        }
                      }}
                      rows={2}
                      style={{
                        width: "100%",
                        background: "var(--btn-bg)",
                        color: "var(--text)",
                        border: "0.5px solid var(--accent-dim)",
                        borderRadius: 6,
                        padding: "4px 7px",
                        fontSize: 12,
                        fontFamily: "inherit",
                        resize: "vertical",
                        lineHeight: 1.5,
                        userSelect: "text",
                        WebkitUserSelect: "text",
                        opacity: isTextSaving ? 0.5 : 1,
                      }}
                    />
                    <span style={{ fontSize: 10, color: "var(--text-dim)" }}>
                      {isTextSaving
                        ? <span className="spinner-rotate" style={{ marginRight: 4 }}>↻</span>
                        : t("workspace.text_edit_hint")}
                    </span>
                  </span>
                ) : (
                  <span
                    style={{ color: "var(--text)", flex: 1, minWidth: 0, cursor: "text" }}
                    title={t("workspace.text_edit_click_hint")}
                    onClick={() =>
                      setSegTextEditing((prev) => ({ ...prev, [segKey]: seg.text }))
                    }
                  >
                    {seg.text}
                  </span>
                )}
                {/* Feature B: supplement badge + toggle */}
                {seg.supplement && (
                  <span
                    className="seg-pill"
                    style={{
                      flexShrink: 0,
                      alignSelf: "center",
                      background: "var(--seg-pill-supplement-bg)",
                      color: "var(--seg-pill-supplement-fg)",
                    }}
                    title={t("workspace.supplement_hint")}
                  >
                    {t("workspace.supplement_label")}
                  </span>
                )}
                <button
                  className="mmr-btn"
                  disabled={isSupplementSaving}
                  onClick={() => toggleSegmentSupplement(seg)}
                  title={t("workspace.supplement_hint")}
                  style={{
                    flexShrink: 0,
                    alignSelf: "center",
                    padding: "2px 7px",
                    fontSize: 10,
                    borderRadius: 6,
                    opacity: seg.supplement ? 1 : 0.45,
                  }}
                >
                  {isSupplementSaving
                    ? <span className="spinner-rotate">↻</span>
                    : t("workspace.supplement_toggle")}
                </button>
              </div>
            );
          })}
        </div>
      )}

      {/* 會議紀錄(摘要)section — 在 reexport 之前(§8.1) */}
      <h4>{t("workspace.summary_title")}</h4>
      <div className="summary-section">
        {/* 後端徽章:客戶版 / 內部版 各自標 */}
        <div className="summary-backends">
          <span className="summary-backend-group">
            <span className="summary-backend-label">{t("workspace.summary_tab_public")}</span>
            {renderBackendBadge(publicBackend)}
          </span>
          <span className="summary-backend-group">
            <span className="summary-backend-label">{t("workspace.summary_tab_internal")}</span>
            {renderBackendBadge(internalBackend)}
          </span>
        </div>

        {/* 強制本地 toggle(知情同意,§5.3) */}
        <label className="summary-force-local">
          <input
            type="checkbox"
            checked={forceLocal}
            onChange={(e) => setForceLocal(e.target.checked)}
            style={{ accentColor: "var(--accent)", cursor: "pointer", flexShrink: 0 }}
          />
          <span>
            <span className="summary-force-local-label">{t("workspace.force_local")}</span>
            <span className="summary-force-local-hint">{t("workspace.force_local_hint")}</span>
          </span>
        </label>

        {/* 生成 / 重新整理 */}
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <button className="mmr-btn primary" onClick={summarize} disabled={summarizing}>
            {summarizing
              ? <><span className="spinner-rotate" style={{ marginRight: 6 }}>↻</span>{t("workspace.summarizing")}</>
              : (summaryPublic || summaryInternal ? t("workspace.summary_reload") : t("workspace.summary_btn"))}
          </button>
          {summaryMsg && <span style={{ fontSize: 11, color: "var(--found-color)" }}>{summaryMsg}</span>}
        </div>
        {summaryErr && (
          <div className="callout" style={{ marginTop: 8, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>
            {summaryErr}
          </div>
        )}

        {/* 客戶版 / 內部版 分頁 */}
        <div className="summary-tabs">
          <button
            className={`tab-btn${summaryTab === "public" ? " active" : ""}`}
            onClick={() => setSummaryTab("public")}
          >
            {t("workspace.summary_tab_public")}
          </button>
          <button
            className={`tab-btn${summaryTab === "internal" ? " active" : ""}`}
            onClick={() => setSummaryTab("internal")}
          >
            {t("workspace.summary_tab_internal")}
          </button>
        </div>

        {summaryTab === "public" ? (
          <div className="summary-body">
            {summaryPublic
              ? <pre className="summary-md">{summaryPublic}</pre>
              : <p className="summary-empty">{t("workspace.summary_empty")}</p>}
          </div>
        ) : (
          <div className="summary-body">
            {summaryInternal
              ? <pre className="summary-md">{summaryInternal}</pre>
              : <p className="summary-empty">{t("workspace.summary_empty")}</p>}
            {/* 內部補充即時預覽:supplement=true 的麥克風段,直接讀記憶體 segments */}
            <div className="summary-supplement-preview">
              <div className="summary-supplement-title">{t("workspace.supplement_preview_title")}</div>
              {supplementSegments.length === 0 ? (
                <p className="summary-empty">{t("workspace.supplement_preview_empty")}</p>
              ) : (
                supplementSegments.map((seg) => (
                  <div key={`${seg.track}-${seg.start_ms}`} className="summary-supplement-line">
                    <span className="summary-supplement-ts">{fmtMs(seg.start_ms)}</span>
                    <span className="summary-supplement-text">{seg.text}</span>
                  </div>
                ))
              )}
            </div>
          </div>
        )}
      </div>

      {/* Re-export section */}
      <h4>{t("workspace.reexport_title")}</h4>
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <button className="mmr-btn" onClick={reexport} disabled={reexporting}>
          {reexporting
            ? <><span className="spinner-rotate" style={{ marginRight: 6 }}>↻</span>{t("workspace.reexporting")}</>
            : t("workspace.reexport_btn")}
        </button>
        {reexportMsg && <span style={{ fontSize: 11, color: "var(--found-color)" }}>{reexportMsg}</span>}
      </div>
      {reexportErr && (
        <div className="callout" style={{ marginTop: 8, color: "var(--danger-color)", borderColor: "rgba(255,99,99,0.3)", background: "rgba(255,99,99,0.08)" }}>
          {reexportErr}
        </div>
      )}
    </div>
  );
}
