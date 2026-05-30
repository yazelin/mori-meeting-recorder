$ErrorActionPreference = "Stop"
# v1.8.4 起有 cuBLAS(GPU)預編版,且 zip 自帶 CUDA runtime(cublas/cudart dll)→ Windows 開 GPU
# 不需要另外裝 CUDA toolkit,直接 drop-in dll 即可。
$ver = if ($env:WHISPER_VERSION) { $env:WHISPER_VERSION } else { "v1.8.4" }
$binDir = "$env:USERPROFILE\.mori\bin"
$modelDir = "$env:USERPROFILE\.mori\models"
New-Item -ItemType Directory -Force -Path $binDir, $modelDir | Out-Null

if (-not (Test-Path "$binDir\whisper-cli.exe")) {
    # 偵測 NVIDIA GPU → 抓 cuBLAS(GPU)版;否則抓 BLAS(CPU)版。
    $gpu = [bool](Get-Command nvidia-smi -ErrorAction SilentlyContinue)
    if ($gpu) {
        # CUDA 12.4 runtime 需要 NVIDIA 驅動 ~550+;舊驅動退回 cuBLAS 11.8(驅動 ~452+)→ 不會因驅動太舊載不起來。
        $drv = (nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>$null | Select-Object -First 1)
        $major = 0; if ($drv -match '^\s*(\d+)') { $major = [int]$Matches[1] }
        if ($major -ge 550) { $zip = "whisper-cublas-12.4.0-bin-x64.zip" } else { $zip = "whisper-cublas-11.8.0-bin-x64.zip" }
        Write-Host "→ 偵測到 NVIDIA GPU(驅動 $drv)→ 下載 GPU(cuBLAS)版 $zip(自帶 CUDA runtime,免裝 toolkit)"
    } else {
        $zip = "whisper-blas-bin-x64.zip"
        Write-Host "→ 無 NVIDIA GPU → 下載 CPU(BLAS)版 whisper.cpp"
    }
    $url = "https://github.com/ggml-org/whisper.cpp/releases/download/$ver/$zip"
    Invoke-WebRequest -Uri $url -OutFile "$env:TEMP\whisper.zip"
    $unzip = "$env:TEMP\whisper-unzip"
    if (Test-Path $unzip) { Remove-Item -Recurse -Force $unzip }
    Expand-Archive -Force "$env:TEMP\whisper.zip" -DestinationPath $unzip

    # 找 whisper-cli.exe(或舊版 main.exe),連同同目錄所有 .dll(含 GPU 版的 ggml-cuda.dll +
    # cublas/cudart runtime)一起複製到 binDir。
    $cli = Get-ChildItem -Path $unzip -Recurse -Include "whisper-cli.exe", "main.exe" | Select-Object -First 1
    if (-not $cli) { Write-Error "zip 裡找不到 whisper-cli.exe / main.exe"; exit 1 }
    $srcDir = $cli.Directory.FullName
    Copy-Item $cli.FullName "$binDir\whisper-cli.exe" -Force
    Copy-Item "$srcDir\*.dll" $binDir -Force
    Remove-Item -Recurse -Force $unzip, "$env:TEMP\whisper.zip"
    Write-Host "✓ installed: $binDir\whisper-cli.exe (+ dlls)"
} else {
    Write-Host "✓ already installed: $binDir\whisper-cli.exe"
}

if (-not (Test-Path "$modelDir\ggml-small.bin")) {
    Write-Host "→ downloading ggml-small model..."
    Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin" `
        -OutFile "$modelDir\ggml-small.bin"
}

# sanity check:當下就抓 DLL 載不起來 / 驅動太舊(GPU 版最容易在這出事),不要等第一次轉錄才爆。
# --help 本身 exit code 不一定 0,所以只在輸出/例外出現「載入失敗」字樣時當錯。
Write-Host ""
Write-Host "==> sanity check"
$sanity = ""
try { $sanity = (& "$binDir\whisper-cli.exe" --help 2>&1 | Out-String) } catch { $sanity = $_.Exception.Message }
if ($sanity -match "0xc000007b|cannot proceed|was not found|無法啟動|找不到") {
    Write-Host "✗ whisper-cli.exe 載不起來(缺 DLL / 驅動版本不夠):"
    Write-Host (($sanity -split "`n") | Select-Object -First 3)
    Write-Host "  GPU 版要夠新的 NVIDIA 驅動;太舊可設 `$env:WHISPER_FORCE_CPU 或改裝 BLAS(CPU)版。"
    exit 1
}
Write-Host "✓ whisper-cli.exe loads OK"

# 檔案轉錄(Files 分頁)用 ffmpeg 把任意音/影格式抽成 16kHz WAV。會議錄音本身不需要 → 裝失敗只警告。
if (Get-Command ffmpeg -ErrorAction SilentlyContinue) {
    Write-Host "✓ ffmpeg already present"
} else {
    Write-Host "→ installing ffmpeg (檔案轉錄用;winget)…"
    try {
        winget install --id Gyan.FFmpeg -e --accept-source-agreements --accept-package-agreements
        Write-Host "✓ ffmpeg installed(可能要開新終端機讓 PATH 生效)"
    } catch {
        Write-Host "⚠ ffmpeg 安裝失敗 — 檔案轉錄(Files 分頁)會用不了;請手動裝 ffmpeg 並加進 PATH。"
    }
}

# 繁體轉換已內建在 app(ferrous-opencc,bundle OpenCC 官方字典)→ 不需要另外裝 opencc。
Write-Host "✓ ready: $binDir\whisper-cli.exe + $modelDir\ggml-small.bin"
Write-Host "  (繁體轉換已內建;large-v3-turbo 模型可在 app 的 Deps 分頁下載)"
Write-Host "  (已裝過要換 CPU↔GPU 版:先刪 $binDir\whisper-cli.exe 再重跑,否則會 skip 不升級)"
