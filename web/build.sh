#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
# Install the wasm-bindgen-cli version matching wasm-bindgen in Cargo.lock.
cargo build --locked --release --lib --target wasm32-unknown-unknown
"${WASM_BINDGEN:-wasm-bindgen}" \
  --target web --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/oneq.wasm
