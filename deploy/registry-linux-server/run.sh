#!/usr/bin/env bash
set -euo pipefail

export RAYTASK_REGISTRY_HOST="${RAYTASK_REGISTRY_HOST:-0.0.0.0}"
export RAYTASK_REGISTRY_PORT="${RAYTASK_REGISTRY_PORT:-8080}"
export RAYTASK_REGISTRY_APP_ROOT="${RAYTASK_REGISTRY_APP_ROOT:-deploy/registry-linux-server/data}"

if [ -x "target/release/raytask" ]; then
  target/release/raytask run apps/registry/main.rt
else
  cargo run -- run apps/registry/main.rt
fi
