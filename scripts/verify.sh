#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
echo "==> npm run build (必須在 cargo check 之前 — generate_context! 需要 dist/)"
npm run build 2>&1 | tail -5
echo "==> cargo test"
(cd src-tauri && cargo test --release 2>&1 | tail -8)
echo "==> cargo check --all-targets"
(cd src-tauri && cargo check --all-targets 2>&1 | tail -3)
echo "✓ verify ok"
