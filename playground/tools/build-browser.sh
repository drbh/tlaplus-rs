#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -z "${WASI_SDK_PATH:-}" ]]; then
  case "$(uname -s)" in Darwin) os=macos ;; Linux) os=linux ;; *) echo 'Set WASI_SDK_PATH to your WASI SDK installation' >&2; exit 1 ;; esac
  case "$(uname -m)" in arm64|aarch64) arch=arm64 ;; x86_64) arch=x86_64 ;; *) echo 'Unsupported CPU' >&2; exit 1 ;; esac
  sdk="wasi-sdk-34.0-$arch-$os"
  mkdir -p .cache
  if [[ ! -x ".cache/$sdk/bin/clang" ]]; then
    curl --fail --location --retry 3 "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/$sdk.tar.gz" -o ".cache/$sdk.tar.gz"
    tar -xzf ".cache/$sdk.tar.gz" -C .cache
  fi
  export WASI_SDK_PATH="$PWD/.cache/$sdk"
fi
python3 tools/prepare-workspace.py
export CARGO_TARGET_DIR="$PWD/../target"
cargo build --manifest-path .cache/browser-workspace/Cargo.toml --locked --release -p tlaplus-playground --target wasm32-wasip1
mkdir -p public
cp ../target/wasm32-wasip1/release/tla-playground.wasm public/checker.wasm
VITE_BROWSER_CHECKER=1 npm run build
