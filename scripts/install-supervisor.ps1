$ErrorActionPreference = "Stop"
# 編譯 + 安裝 recorder 的兩支 headless sidecar 到 %USERPROFILE%\.mori\bin\,讓**任何 app**
# (recorder / mori-desktop / AgentOS / shell)都能一行隨需喚醒:
#
#   - mori-whisper-serve.exe   : 共享 whisper-server 的 supervisor / idle-reaper(本地 STT)
#   - mori-summarize-serve.exe : 會議雙摘要 pipeline 的 HTTP sidecar
#                                (AgentOS dispatch meeting.summarize → 真 pipeline 走它)
#
#     %USERPROFILE%\.mori\bin\mori-whisper-serve.exe   --ensure   # 沒在跑就背景拉起,冪等、馬上返回
#     %USERPROFILE%\.mori\bin\mori-summarize-serve.exe --ensure   # 同上(AgentOS dispatch 前喚醒摘要服務)
#     %USERPROFILE%\.mori\bin\<name>.exe               --stop      # 立刻停掉
#
# 兩者閒置都會自關(各 10 分鐘)。契約見 agentos-notebook/05-mori-migration/whisper-server-contract.md §11。
#
# 註:recorder 第一次跑時也會 best-effort 把 sibling sidecar 種進 ~\.mori\bin —— **但那只在 dev 成立**;
# packaged bundle 沒把 sidecar 列為 Tauri externalBin,所以**打包/部署後請用本腳本鋪**。

$binDir = "$env:USERPROFILE\.mori\bin"
$root = (Resolve-Path "$PSScriptRoot\..").Path
New-Item -ItemType Directory -Force -Path $binDir | Out-Null

function Install-Bin($name) {
    Write-Host "-> cargo build --release --bin $name..."
    Push-Location "$root\src-tauri"
    try {
        cargo build --release --bin $name
    } finally {
        Pop-Location
    }
    $built = "$root\src-tauri\target\release\$name.exe"
    if (-not (Test-Path $built)) {
        Write-Error "build 沒產出 $built"
        exit 1
    }
    Copy-Item $built "$binDir\$name.exe" -Force
    Write-Host "✓ installed: $binDir\$name.exe"
}

Install-Bin "mori-whisper-serve"
Install-Bin "mori-summarize-serve"

Write-Host ""
Write-Host "喚醒/停止:"
Write-Host "  $binDir\mori-whisper-serve.exe   --ensure | --stop"
Write-Host "  $binDir\mori-summarize-serve.exe --ensure | --stop"
