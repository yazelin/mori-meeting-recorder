// src/components/RecordButton.tsx
//
// Filled-style action button — 三個 state 對應 mock 01(stop)/ 02(start)/ 03(transcribing)。
// 跟 CapsuleView 既有 `.icon-btn` 並列,不替換它。

import { useTranslation } from "react-i18next";
import TriangleIcon from "./icons/TriangleIcon";
import SquareIcon from "./icons/SquareIcon";
import SpinnerIcon from "./icons/SpinnerIcon";

type State = "idle" | "recording" | "transcribing";

interface Props {
  state: State;
  onClick: () => void;
}

export default function RecordButton({ state, onClick }: Props) {
  const { t } = useTranslation();
  const disabled = state === "transcribing";
  const title =
    state === "recording"   ? t("capsule.stop")  :
    state === "transcribing" ? t("capsule.transcribing") :
                               t("capsule.start");

  return (
    <button
      type="button"
      className="record-btn"
      data-state={state}
      onClick={onClick}
      disabled={disabled}
      title={title}
    >
      {state === "idle"        && <TriangleIcon size={12} />}
      {state === "recording"   && <SquareIcon   size={10} />}
      {state === "transcribing" && <SpinnerIcon size={14} />}
    </button>
  );
}
