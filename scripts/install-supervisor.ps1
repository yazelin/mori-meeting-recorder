$ErrorActionPreference = "Stop"
# 編譯 mori-whisper-serve(共享 whisper-server 的 supervisor / idle-reaper)並安裝到
# %USERPROFILE%\.mori\bin\,讓**任何 app**(recorder / mori-desktop / AgentOS / shell)
# 都能用一行指令隨需喚醒本地 whisper 服務:
#
#     %USERPROFILE%\.mori\bin\mori-whisper-serve.exe --ensure   # 沒在跑就背景拉起,冪等、馬上返回
#     %USERPROFILE%\.mori\bin\mori-whisper-serve.exe --stop      # 立刻停掉
#
# server 閒置(無 /inference)10 分鐘會自己關。契約見
# agentos-notebook/05-mori-migration/whisper-server-contract.md §11。
#
# 註:recorder 第一次執行時 ensure_server() 也會 best-effort 把 supervisor 種進 ~\.mori\bin。

$binDir = "$env:USERPROFILE\.mori\bin"
$root = (Resolve-Path "$PSScriptRoot\..").Path
New-Item -ItemType Directory -Force -Path $binDir | Out-Null

Write-Host "-> cargo build --release --bin mori-whisper-serve..."
Push-Location "$root\src-tauri"
try {
    cargo build --release --bin mori-whisper-serve
} finally {
    Pop-Location
}

$built = "$root\src-tauri\target\release\mori-whisper-serve.exe"
if (-not (Test-Path $built)) {
    Write-Error "build 沒產出 $built"
    exit 1
}

Copy-Item $built "$binDir\mori-whisper-serve.exe" -Force
Write-Host "✓ installed: $binDir\mori-whisper-serve.exe"
Write-Host "  喚醒:$binDir\mori-whisper-serve.exe --ensure"
Write-Host "  停止:$binDir\mori-whisper-serve.exe --stop"
