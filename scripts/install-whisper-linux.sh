#!/usr/bin/env bash
# 從 whisper.cpp source 編譯 whisper-cli + 共用 .so libs 到 ~/.mori/bin/。
# 走 source-build 是因為 v1.7+ 官方 release 不再 ship Linux x64 prebuilt zip
# (release zip URL → 404 → 9 bytes 假 zip → unzip 爆掉)。對齊 mori-desktop
# 的 whisper-server install 邏輯,共享 ~/.mori/bin/lib*.so。
#
# Requires: git, cmake, make/ninja, g++/clang++(`sudo apt install build-essential cmake`)

set -euo pipefail

version="${WHISPER_VERSION:-v1.8.4}"
bin_dir="$HOME/.mori/bin"
model_dir="$HOME/.mori/models"
work="/tmp/mori-whisper-cli-${version}"

mkdir -p "$bin_dir" "$model_dir"

# 1. whisper-cli binary
if [ -x "$bin_dir/whisper-cli" ]; then
  echo "✓ already installed: $bin_dir/whisper-cli"
else
  echo "→ building whisper.cpp $version from source…"
  rm -rf "$work" && mkdir -p "$work" && cd "$work"
  curl -L -o whisper.cpp.tar.gz \
    "https://github.com/ggml-org/whisper.cpp/archive/refs/tags/${version}.tar.gz"
  tar -xzf whisper.cpp.tar.gz
  src_dir="whisper.cpp-${version#v}"
  # GPU:偵測到 CUDA toolkit(nvcc)就用 GPU 編(GGML_CUDA=1)→ whisper-cli 會吃 NVIDIA GPU。
  # 沒 nvcc 就 CPU 編。要 GPU 加速但缺 nvcc:先 `sudo apt install nvidia-cuda-toolkit` 再重跑本指令。
  cuda_flag=""
  if command -v nvcc >/dev/null 2>&1; then
    echo "  ✓ 偵測到 CUDA toolkit → 用 GPU 編譯(GGML_CUDA=1)"
    cuda_flag="-DGGML_CUDA=1"
  else
    echo "  · 無 CUDA toolkit(nvcc)→ CPU 編譯。要 GPU 請先 sudo apt install nvidia-cuda-toolkit 再重跑。"
  fi
  cmake -S "$src_dir" -B build \
    -DCMAKE_BUILD_TYPE=Release \
    -DWHISPER_BUILD_TESTS=OFF \
    -DWHISPER_BUILD_EXAMPLES=ON \
    -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
    -DCMAKE_INSTALL_RPATH='$ORIGIN' \
    $cuda_flag
  cmake --build build --target whisper-cli -j"$(nproc)"
  # whisper.cpp v1.7+ binary 叫 whisper-cli;舊版叫 main。
  if [ -x build/bin/whisper-cli ]; then
    cp -f build/bin/whisper-cli "$bin_dir/whisper-cli"
  elif [ -x build/bin/main ]; then
    cp -f build/bin/main "$bin_dir/whisper-cli"
  else
    echo "✗ no whisper-cli or main binary in build/bin/" >&2
    exit 1
  fi
  chmod +x "$bin_dir/whisper-cli"
  # 共用 shared libs(可能 mori-desktop 已經放過,我們覆蓋確保版本一致)
  if compgen -G "build/src/libwhisper.so.*" > /dev/null; then
    cp -f build/src/libwhisper.so.* "$bin_dir/" || true
  fi
  # libggml*.so:CPU/base 在 build/ggml/src/,但 CUDA backend(libggml-cuda.so)在子目錄
  # build/ggml/src/ggml-cuda/ → 用 find 遞迴抓,不然 GPU build 會漏掉 cuda lib(whisper-cli
  # 會 error: libggml-cuda.so.0 cannot open)。
  find build -name 'libggml*.so.*' -exec cp -f {} "$bin_dir/" \; 2>/dev/null || true
  # 把 versioned .so symlink 起來,讓 whisper-cli 用 RPATH=$ORIGIN 找得到
  cd "$bin_dir"
  if compgen -G "libwhisper.so.*.*.*" > /dev/null; then
    ln -sf "$(ls libwhisper.so.*.*.* | sort -V | tail -1)" libwhisper.so.1
    ln -sf libwhisper.so.1 libwhisper.so
  fi
  if compgen -G "libggml.so.*.*.*" > /dev/null; then
    ln -sf "$(ls libggml.so.*.*.* | sort -V | tail -1)" libggml.so.0
    ln -sf libggml.so.0 libggml.so
  fi
  if compgen -G "libggml-base.so.*.*.*" > /dev/null; then
    ln -sf "$(ls libggml-base.so.*.*.* | sort -V | tail -1)" libggml-base.so.0
  fi
  if compgen -G "libggml-cpu.so.*.*.*" > /dev/null; then
    ln -sf "$(ls libggml-cpu.so.*.*.* | sort -V | tail -1)" libggml-cpu.so.0
  fi
  # GPU build 才有:CUDA backend lib 的 SONAME symlink(whisper-cli 找的是 libggml-cuda.so.0)
  if compgen -G "libggml-cuda.so.*.*.*" > /dev/null; then
    ln -sf "$(ls libggml-cuda.so.*.*.* | sort -V | tail -1)" libggml-cuda.so.0
  fi
  # 診斷:GPU backend lib 有沒有進 bin(沒進 = CPU build,或 build 時 nvcc 不在 PATH)
  if compgen -G "$bin_dir/libggml-cuda.so*" > /dev/null; then
    echo "  ✓ GPU backend lib: $(ls "$bin_dir"/libggml-cuda.so* | xargs -n1 basename | tr '\n' ' ')"
  else
    echo "  · CPU build(無 libggml-cuda)。要 GPU:確認 nvcc 在 PATH(which nvcc)後重跑。"
  fi
  rm -rf "$work"
  echo "✓ built: $bin_dir/whisper-cli"
fi

# 2. ggml-small model(465 MB,whisper.cpp 標準 model)
if [ -f "$model_dir/ggml-small.bin" ]; then
  echo "✓ already installed: $model_dir/ggml-small.bin"
else
  echo "→ downloading ggml-small model (~465MB)…"
  curl -L --fail -o "$model_dir/ggml-small.bin" \
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
  echo "✓ downloaded: $model_dir/ggml-small.bin"
fi

# 繁體轉換已內建在 app(ferrous-opencc,bundle OpenCC 官方字典)→ 不再需要外部 opencc。

# sanity test
echo ""
echo "==> sanity check"
"$bin_dir/whisper-cli" --help 2>&1 | head -2 || {
  echo "✗ whisper-cli 跑不起來 — 可能 shared lib RPATH 沒接好。試 LD_LIBRARY_PATH=$bin_dir ldd $bin_dir/whisper-cli"
  exit 1
}
echo ""
echo "✓ ready: $bin_dir/whisper-cli + $model_dir/ggml-small.bin"
