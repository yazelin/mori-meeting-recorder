#!/usr/bin/env bash
# 編譯 mori-whisper-serve(共享 whisper-server 的 supervisor / idle-reaper)並安裝到
# ~/.mori/bin/,讓**任何 app**(recorder / mori-desktop / AgentOS / python mori-ear / shell)
# 都能用一行指令隨需喚醒本地 whisper 服務:
#
#     ~/.mori/bin/mori-whisper-serve --ensure   # 沒在跑就背景拉起,冪等、馬上返回
#     ~/.mori/bin/mori-whisper-serve --stop      # 立刻停掉
#
# server 閒置(無 /inference)10 分鐘會自己關。契約見
# agentos-notebook/05-mori-migration/whisper-server-contract.md §11。
#
# 註:recorder 第一次執行時 ensure_server() 也會 best-effort 把 supervisor 種進 ~/.mori/bin,
# 所以「跑過一次 recorder」也能自動鋪好;本腳本給「還沒跑過 recorder 就想讓別的 app 用」的情境。

set -euo pipefail

bin_dir="$HOME/.mori/bin"
root="$(cd "$(dirname "$0")/.." && pwd)"

mkdir -p "$bin_dir"

echo "→ cargo build --release --bin mori-whisper-serve…"
( cd "$root/src-tauri" && cargo build --release --bin mori-whisper-serve )

built="$root/src-tauri/target/release/mori-whisper-serve"
if [ ! -x "$built" ]; then
  echo "✗ build 沒產出 $built" >&2
  exit 1
fi

# 寫 .tmp 再 rename 覆蓋 —— rename 蓋過「正在被 exec 的舊 binary」在 Linux 安全(避免 ETXTBSY)。
tmp="$bin_dir/mori-whisper-serve.tmp-install"
cp -f "$built" "$tmp"
chmod +x "$tmp"
mv -f "$tmp" "$bin_dir/mori-whisper-serve"

echo "✓ installed: $bin_dir/mori-whisper-serve"
echo "  喚醒:$bin_dir/mori-whisper-serve --ensure"
echo "  停止:$bin_dir/mori-whisper-serve --stop"
