// src/tabs/LiveTab.tsx
//
// 雙欄即時字幕(presentational)。字幕狀態 + live-segment 監聽器住在 App.tsx,
// LiveTab 只負責顯示 —— 這樣切分頁/收合卸載 LiveTab 也不會丟字幕。

import { useTranslation } from "react-i18next";
import LiveColumn, { type LiveSegment } from "../components/LiveColumn";

export default function LiveTab({ sys, mic }: { sys: LiveSegment[]; mic: LiveSegment[] }) {
  const { t } = useTranslation();
  const empty = sys.length === 0 && mic.length === 0;
  return (
    <div>
      {empty && <p style={{ color: "var(--text-dim)", fontSize: 12 }}>{t("live.empty")}</p>}
      <div className="live-cols">
        <LiveColumn title={t("live.col_sys")} segments={sys} />
        <LiveColumn title={t("live.col_mic")} segments={mic} />
      </div>
    </div>
  );
}
