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

  // Speaker rename pending map: id -> display value being edited
  const [speakerEdits, setSpeakerEdits] = useState<Record<string, string>>({});
  const [speakerSaving, setSpeakerSaving] = useState<Record<string, boolean>>({});

  // Speaker merge selection
  const [selectedSpeakers, setSelectedSpeakers] = useState<string[]>([]);
  const [merging, setMerging] = useState(false);
  const [mergeErr, setMergeErr] = useState<string | null>(null);

  // Per-segment speaker reassign loading
  const [segSaving, setSegSaving] = useState<Record<string, boolean>>({});

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
    const key = `${seg.track}-${seg.id}`;
    setSegSaving((prev) => ({ ...prev, [key]: true }));
    try {
      await invoke("set_segment_speaker", {
        sessionId,
        track: seg.track,
        segId: seg.id,
        speakerId,
      });
      await reloadTranscript();
    } catch (e: any) {
      console.error("set_segment_speaker:", e);
    } finally {
      setSegSaving((prev) => ({ ...prev, [key]: false }));
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
    <div>
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

      {/* Transcript section */}
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
            maxHeight: 280,
            overflowY: "auto",
          }}
        >
          {segments.map((seg) => {
            const segKey = `${seg.track}-${seg.id}`;
            const display = seg.speaker ? (speakerDisplay[seg.speaker] ?? seg.speaker) : t("workspace.unknown_speaker");
            const isSaving = segSaving[segKey] ?? false;
            return (
              <div key={segKey} style={{ display: "flex", gap: 8, marginBottom: 6, fontSize: 12, lineHeight: 1.5, alignItems: "flex-start" }}>
                <span style={{ fontFamily: "ui-monospace, monospace", fontSize: 10, color: "var(--text-dim)", flexShrink: 0, paddingTop: 3 }}>
                  {fmtMs(seg.start_ms)}
                </span>
                {/* Per-segment speaker Select (only when speakers exist) */}
                {speakerOptions.length > 0 ? (
                  <span style={{ flexShrink: 0, position: "relative" }}>
                    <Select
                      value={seg.speaker ?? ""}
                      options={speakerOptions}
                      onChange={(newId) => setSegmentSpeaker(seg, newId)}
                    />
                    {isSaving && (
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
                <span style={{ color: "var(--text)" }}>{seg.text}</span>
              </div>
            );
          })}
        </div>
      )}

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
