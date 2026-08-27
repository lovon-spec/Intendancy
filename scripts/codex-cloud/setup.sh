#!/usr/bin/env bash
set -euo pipefail

task_sudo=()
if [ "$(id -u)" -ne 0 ]; then
  task_sudo=(sudo -n)
fi

if command -v apt-get >/dev/null 2>&1; then
  "${task_sudo[@]}" apt-get update
  "${task_sudo[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    git \
    gzip \
    jq \
    libdigest-sha-perl \
    libssl-dev \
    pkg-config \
    zsh
fi

export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
if [ -s "$NVM_DIR/nvm.sh" ]; then
  # Node 25.8.2 is the version pinned by the descriptor-vector evidence.
  # It also exceeds Vite 8's Node >=22.12 requirement.
  # shellcheck source=/dev/null
  . "$NVM_DIR/nvm.sh"
  nvm install 25.8.2
  nvm alias default 25.8.2
  nvm use 25.8.2
  corepack enable
else
  echo "Codex universal image is missing nvm at $NVM_DIR/nvm.sh" >&2
  exit 1
fi

rustup toolchain install 1.94.0 --profile minimal -c rustfmt,clippy
rustup default 1.94.0

export PATH="$HOME/.foundry/bin:$PATH"
if ! command -v foundryup >/dev/null 2>&1; then
  curl -fsSL https://foundry.paradigm.xyz | bash
fi
if ! command -v forge >/dev/null 2>&1 || ! forge --version | grep -Fq 'Version: 1.5.1-stable'; then
  "$HOME/.foundry/bin/foundryup" --install 1.5.1
fi

touch "$HOME/.bashrc"
# Keep the variables literal so they expand in each future agent shell.
# shellcheck disable=SC2016
grep -Fqx 'export PATH="$HOME/.foundry/bin:$PATH"' "$HOME/.bashrc" || \
  printf '%s\n' 'export PATH="$HOME/.foundry/bin:$PATH"' >> "$HOME/.bashrc"

git submodule sync --recursive
git submodule update --init --recursive
npm ci --prefix frontend --prefer-offline --no-audit --no-fund
cargo fetch --locked --manifest-path spikes/gnosis-anchor/Cargo.toml
forge build --root contracts
