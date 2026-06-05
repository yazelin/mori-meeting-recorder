#!/usr/bin/env bash
# 編譯 + 安裝 recorder 的兩支 headless sidecar 到 ~/.mori/bin/,讓**任何 app**
# (recorder / mori-desktop / AgentOS / python mori-ear / shell)都能一行隨需喚醒:
#
#   - mori-whisper-serve   : 共享 whisper-server 的 supervisor / idle-reaper(本地 STT)
#   - mori-summarize-serve : 會議雙摘要 pipeline 的 HTTP sidecar
#                            (AgentOS dispatch meeting.summarize → 真 pipeline 走它)
#
#     ~/.mori/bin/mori-whisper-serve   --ensure   # 沒在跑就背景拉起,冪等、馬上返回
#     ~/.mori/bin/mori-summarize-serve --ensure   # 同上(AgentOS dispatch 前喚醒摘要服務)
#     ~/.mori/bin/<name>               --stop      # 立刻停掉
#
# 兩者閒置都會自關(whisper 無 /inference / summarize 無請求,各 10 分鐘)。契約見
# agentos-notebook/05-mori-migration/whisper-server-contract.md §11。
#
# 註:recorder 第一次跑時也會 best-effort 把 sibling sidecar 種進 ~/.mori/bin —— **但那只在 dev
# 成立**(sidecar bin 在 app 旁邊時)。packaged bundle 沒把 sidecar 列為 Tauri externalBin,
# 所以**打包/部署後請用本腳本鋪**(這也是「還沒跑過 recorder 就想讓 AgentOS dispatch」的情境)。

set -euo pipefail

bin_dir="$HOME/.mori/bin"
root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$bin_dir"

install_bin() {
  local name="$1"
  echo "→ cargo build --release --bin $name…"
  ( cd "$root/src-tauri" && cargo build --release --bin "$name" )
  local built="$root/src-tauri/target/release/$name"
  if [ ! -x "$built" ]; then
    echo "✗ build 沒產出 $built" >&2
    exit 1
  fi
  # 寫 .tmp 再 rename 覆蓋 —— rename 蓋過「正在被 exec 的舊 binary」在 Linux 安全(避免 ETXTBSY)。
  local tmp="$bin_dir/$name.tmp-install"
  cp -f "$built" "$tmp"
  chmod +x "$tmp"
  mv -f "$tmp" "$bin_dir/$name"
  echo "✓ installed: $bin_dir/$name"
}

install_bin mori-whisper-serve
install_bin mori-summarize-serve

echo
echo "喚醒/停止:"
echo "  $bin_dir/mori-whisper-serve   --ensure | --stop   # 本地 whisper STT"
echo "  $bin_dir/mori-summarize-serve --ensure | --stop   # 會議雙摘要(AgentOS dispatch)"
