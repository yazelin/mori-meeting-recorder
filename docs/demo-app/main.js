const shell = document.querySelector(".demo-shell");
const capsule = document.querySelector(".capsule");
const capsuleDot = document.querySelector(".capsule-dot");
const capsuleStatus = document.querySelector(".capsule-status");
const capsuleTime = document.querySelector(".capsule-time");
const controlDot = document.querySelector(".control-dot");
const controlLabel = document.querySelector(".control-label");
const controlTime = document.querySelector(".control-time");
const recordButtons = document.querySelectorAll(".record-toggle");
const ccToggle = document.querySelector(".cc-toggle");
const workspace = document.querySelector(".workspace");
const summaryCard = document.querySelector("[data-summary-card]");

let state = "idle";
let elapsed = 0;
let mode = "online";
let done = { sys: 0, mic: 0 };
let pending = { sys: 0, mic: 0 };

const sysLines = [
  "客戶希望會後拿到一份可直接轉寄的會議紀錄。",
  "本次導入先以線上會議模式測試雙軌錄音。",
  "下週會確認資料保存位置和內部權限。",
  "正式版輸出需要區分客戶版與內部版。"
];

const micLines = [
  "內部補充：報價細節先不要放入客戶版。",
  "記得會後確認 whisper large-v3-turbo 模型。",
  "這段是團隊旁白，只進 internal transcript。",
  "下次 demo 前先測 USB 會議麥克風。"
];

function fmt(seconds) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function setState(next) {
  state = next;
  shell.dataset.state = next;
  capsule.dataset.state = next;
  [capsuleDot, controlDot].forEach((el) => {
    el.classList.remove("idle", "recording", "transcribing");
    el.classList.add(next);
  });
  [capsuleStatus, controlLabel].forEach((el) => {
    el.classList.remove("idle", "recording", "transcribing");
    el.classList.add(next);
  });

  const label = next === "recording" ? "REC" : next === "transcribing" ? "轉錄中" : "idle";
  capsuleStatus.textContent = label;
  controlLabel.textContent = label;

  recordButtons.forEach((button) => {
    button.disabled = next === "transcribing";
    button.title = next === "recording" ? "停止錄音" : "開始錄音";
    button.setAttribute("aria-label", button.title);
    button.innerHTML = next === "recording"
      ? '<span class="stop-icon"></span>'
      : next === "transcribing"
        ? '<span class="spinner-icon"></span>'
        : '<span class="play-icon"></span>';
  });

  document.querySelectorAll(".signal-pill").forEach((pill) => {
    pill.classList.toggle("on", next === "recording");
  });
}

function setElapsed(next) {
  elapsed = next;
  capsuleTime.textContent = fmt(elapsed);
  controlTime.textContent = fmt(elapsed);
}

function makeBars() {
  document.querySelectorAll(".vu-meter").forEach((meter) => {
    meter.innerHTML = "";
    for (let i = 0; i < 24; i += 1) {
      const bar = document.createElement("span");
      bar.className = "vu-bar";
      meter.appendChild(bar);
    }
  });
}

function animateBars() {
  document.querySelectorAll(".vu-meter").forEach((meter) => {
    const bars = [...meter.children];
    bars.forEach((bar, index) => {
      const active = state === "recording";
      const wave = Math.sin((Date.now() / 130) + index * 0.7);
      const level = active ? Math.max(0, Math.round((wave + 1) * 10 + Math.random() * 4)) : 0;
      bar.style.height = `${Math.max(6, level + 6)}px`;
      bar.classList.toggle("on", active && level > 7);
      bar.classList.toggle("peak", active && level > 18);
    });
  });
}

function addSegment(track, text, seconds) {
  const list = document.querySelector(`[data-segments="${track}"]`);
  if (!list) return;
  const segment = document.createElement("div");
  segment.className = "segment";
  segment.innerHTML = `<time>${fmt(seconds)}</time>${text}`;
  list.prepend(segment);
  while (list.children.length > 5) list.lastElementChild.remove();
}

function updateCounters() {
  ["sys", "mic"].forEach((track) => {
    document.querySelector(`[data-done="${track}"]`).textContent = done[track];
    document.querySelector(`[data-pending="${track}"]`).textContent = pending[track];
  });
}

recordButtons.forEach((button) => {
  button.addEventListener("click", () => {
    if (state === "idle") {
      setElapsed(0);
      done = { sys: 0, mic: 0 };
      pending = { sys: 0, mic: 0 };
      updateCounters();
      setState("recording");
      shell.classList.add("captions-on");
      ccToggle.classList.add("active");
      return;
    }

    if (state === "recording") {
      setState("transcribing");
      pending = mode === "online" ? { sys: 2, mic: 1 } : { sys: 0, mic: 2 };
      updateCounters();
      setTimeout(() => {
        done.sys += pending.sys;
        done.mic += pending.mic;
        pending = { sys: 0, mic: 0 };
        updateCounters();
        setState("idle");
      }, 1700);
    }
  });
});

document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((item) => item.classList.toggle("active", item === tab));
    document.querySelectorAll(".tab-panel").forEach((panel) => {
      panel.classList.toggle("active", panel.dataset.panel === tab.dataset.tab);
    });
  });
});

document.querySelectorAll("[data-mode]").forEach((button) => {
  button.addEventListener("click", () => {
    if (state !== "idle") return;
    mode = button.dataset.mode;
    document.querySelectorAll("[data-mode]").forEach((item) => item.classList.toggle("primary", item === button));
    document.querySelector(".system-device").style.display = mode === "online" ? "flex" : "none";
    document.querySelector(".track-system").style.display = mode === "online" ? "block" : "none";
  });
});

ccToggle.addEventListener("click", () => {
  shell.classList.toggle("captions-on");
  ccToggle.classList.toggle("active", shell.classList.contains("captions-on"));
});

document.querySelector("[data-open-workspace]").addEventListener("click", () => {
  workspace.classList.add("open");
});

document.querySelector(".close-workspace").addEventListener("click", () => {
  workspace.classList.remove("open");
});

document.querySelectorAll("[data-summary]").forEach((button) => {
  button.addEventListener("click", () => {
    document.querySelectorAll("[data-summary]").forEach((item) => item.classList.toggle("active", item === button));
    const internal = button.dataset.summary === "internal";
    summaryCard.innerHTML = internal
      ? `<h4>內部補充</h4><ul><li>報價與風險評估保留在 internal summary。</li><li>技術顧問建議先建立測試資料夾與模型快取。</li><li>會後由內部 PM 確認下一輪 PoC 範圍。</li></ul>`
      : `<h4>會議重點</h4><ul><li>確認採用本機 whisper 轉錄，敏感會議強制本機摘要。</li><li>交付文件使用客戶版，內部旁白不混入對外紀錄。</li><li>下次會議前完成 USB 會議麥克風測試。</li></ul>`;
  });
});

setInterval(() => {
  animateBars();
  if (state !== "recording") return;

  setElapsed(elapsed + 1);
  if (elapsed % 4 === 1) {
    const line = sysLines[Math.floor(elapsed / 4) % sysLines.length];
    addSegment("sys", line, elapsed);
    done.sys += 1;
  }
  if (elapsed % 5 === 2) {
    const line = micLines[Math.floor(elapsed / 5) % micLines.length];
    addSegment("mic", line, elapsed);
    done.mic += 1;
  }
  pending.sys = elapsed % 4 === 0 ? 1 : 0;
  pending.mic = elapsed % 5 === 0 ? 1 : 0;
  updateCounters();
}, 1000);

setInterval(animateBars, 140);

makeBars();
setState("idle");
setElapsed(0);
updateCounters();
addSegment("sys", sysLines[0], 12);
addSegment("mic", micLines[0], 17);
