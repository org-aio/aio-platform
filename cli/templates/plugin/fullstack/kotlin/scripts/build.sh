#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
KOTLIN_CLI_NO_WELCOME_BANNER=1 ./kotlin test -m shared -p jvm
KOTLIN_CLI_NO_WELCOME_BANNER=1 ./kotlin test -m service -p jvm

KOTLIN_CLI_NO_WELCOME_BANNER=1 ./kotlin build -m frontend -p wasmJs -v release
KOTLIN_CLI_NO_WELCOME_BANNER=1 ./kotlin package -m service -p jvm -f executable-jar
mkdir -p dist
rm -rf dist/frontend
mkdir -p dist/frontend
cp -R build/tasks/_frontend_buildWasmJsAppWasmJsRelease/. dist/frontend/
cp build/tasks/_service_executableJarJvm/service-jvm-executable.jar dist/plugin.jar
