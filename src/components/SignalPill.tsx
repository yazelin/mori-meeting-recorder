// src/components/SignalPill.tsx
//
// 取代 CapsuleView 中既有的 inline `<span className="signal-pill">...<span className="signal-pill-dot" />SYS</span>`。
// 圖示從圓點換成 BarsIcon,符合 mock 01/02/04 v2 的視覺。
// Active 時 .on 套既有 theme.css rule(綠底綠字);err 時 .err 套(紅底紅字)。

import { useTranslation } from "react-i18next";
import BarsIcon from "./icons/BarsIcon";

type Kind = "sys" | "mic";

interface Props {
  kind: Kind;
  active: boolean;
}

const LABEL: Record<Kind, string> = { sys: "SYS", mic: "MIC" };
const TITLE_KEY: Record<Kind, string> = {
  sys: "capsule.system_pill",
  mic: "capsule.mic_pill",
};

export default function SignalPill({ kind, active }: Props) {
  const { t } = useTranslation();
  return (
    <span className={`signal-pill${active ? " on" : ""}`} title={t(TITLE_KEY[kind])}>
      <BarsIcon size={10} />
      {LABEL[kind]}
    </span>
  );
}
