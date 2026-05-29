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
        $zip = "whisper-cublas-12.4.0-bin-x64.zip"
        Write-Host "→ 偵測到 NVIDIA GPU → 下載 GPU(cuBLAS)版 whisper.cpp(~436MB,內含 CUDA runtime,免裝 CUDA toolkit)"
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

# 繁體轉換已內建在 app(ferrous-opencc,bundle OpenCC 官方字典)→ 不需要另外裝 opencc。
Write-Host "✓ ready: $binDir\whisper-cli.exe + $modelDir\ggml-small.bin"
Write-Host "  (繁體轉換已內建;large-v3-turbo 模型可在 app 的 Deps 分頁下載)"
