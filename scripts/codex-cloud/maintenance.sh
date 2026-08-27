#!/usr/bin/env bash
set -euo pipefail

export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
if [ -s "$NVM_DIR/nvm.sh" ]; then
  # shellcheck source=/dev/null
  . "$NVM_DIR/nvm.sh"
  nvm use 25.8.2
fi
export PATH="$HOME/.foundry/bin:$PATH"

git submodule sync --recursive
git submodule update --init --recursive

if [ -f frontend/package-lock.json ]; then
  npm ci --prefix frontend --prefer-offline --no-audit --no-fund
fi

for manifest in \
  cli/Cargo.toml \
  spikes/gnosis-anchor/Cargo.toml \
  spikes/snapshot-bench/Cargo.toml
do
  if [ -f "$manifest" ]; then
    cargo fetch --locked --manifest-path "$manifest"
  fi
done

forge build --root contracts
