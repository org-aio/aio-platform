#!/bin/sh
set -eu
mkdir -p dist/web/assets
if [ "$1" = backend ]; then
    cargo build -p fullstack-backend --target wasm32-unknown-unknown
    wasm-tools component new target/wasm32-unknown-unknown/debug/fullstack_backend.wasm -o dist/backend.wasm
else
    aio plugin dev-build rust-frontend
fi
