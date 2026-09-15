#!/bin/sh
set -eu
pnpm install --frozen-lockfile
pnpm build
pnpm typecheck
pnpm test
