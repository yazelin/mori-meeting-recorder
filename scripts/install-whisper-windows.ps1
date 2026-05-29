$ErrorActionPreference = "Stop"
$ver = if ($env:WHISPER_VERSION) { $env:WHISPER_VERSION } else { "v1.7.0" }
$binDir = "$env:USERPROFILE\.mori\bin"
$modelDir = "$env:USERPROFILE\.mori\models"
New-Item -ItemType Directory -Force -Path $binDir, $modelDir | Out-Null
if (-not (Test-Path "$binDir\whisper-cli.exe")) {
    Write-Host "→ downloading whisper.cpp $ver..."
    $url = "https://github.com/ggerganov/whisper.cpp/releases/download/$ver/whisper-blas-bin-x64.zip"
    Invoke-WebRequest -Uri $url -OutFile "$env:TEMP\whisper.zip"
    Expand-Archive -Force "$env:TEMP\whisper.zip" -DestinationPath "$env:TEMP\whisper-unzip"
    if (Test-Path "$env:TEMP\whisper-unzip\main.exe") {
        Copy-Item "$env:TEMP\whisper-unzip\main.exe" "$binDir\whisper-cli.exe"
    } else {
        Copy-Item "$env:TEMP\whisper-unzip\whisper-cli.exe" "$binDir\whisper-cli.exe"
    }
    Remove-Item -Recurse "$env:TEMP\whisper-unzip", "$env:TEMP\whisper.zip"
}
if (-not (Test-Path "$modelDir\ggml-small.bin")) {
    Write-Host "→ downloading ggml-small model..."
    Invoke-WebRequest -Uri "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin" `
        -OutFile "$modelDir\ggml-small.bin"
}

# OpenCC (optional — Traditional Chinese conversion)
# opencc on Windows is not available via winget. Install via pip if Python is present,
# or download a prebuilt binary from https://github.com/BYVoid/OpenCC/releases.
# The Rust side gracefully skips Traditional conversion if opencc is absent — non-fatal.
Write-Host ""
Write-Host "==> opencc (optional, for Traditional Chinese conversion)"
if (Get-Command opencc -ErrorAction SilentlyContinue) {
    Write-Host "opencc already on PATH"
} elseif (Get-Command pip -ErrorAction SilentlyContinue) {
    Write-Host "Attempting: pip install opencc-python-reimplemented (provides opencc CLI)"
    pip install opencc-python-reimplemented 2>&1 | Out-Null
    if (Get-Command opencc -ErrorAction SilentlyContinue) {
        Write-Host "opencc installed via pip"
    } else {
        Write-Host "pip install did not place opencc on PATH. Traditional conversion will be skipped at runtime."
        Write-Host "Manual option: download a prebuilt opencc binary and place it at $binDir\opencc.exe"
    }
} else {
    Write-Host "opencc not found and pip not available."
    Write-Host "Traditional conversion will be skipped at runtime (non-fatal)."
    Write-Host "To enable: download opencc from https://github.com/BYVoid/OpenCC/releases"
    Write-Host "           and place opencc.exe at $binDir\opencc.exe"
}

Write-Host "✓ ready: $binDir\whisper-cli.exe + $modelDir\ggml-small.bin"
