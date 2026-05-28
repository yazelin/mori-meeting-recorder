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
Write-Host "✓ ready: $binDir\whisper-cli.exe + $modelDir\ggml-small.bin"
