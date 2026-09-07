#!/usr/bin/env sh
# Install the intend CLI and the Agent Skills Registry profile from a signed
# GitHub release, verifying everything against the release signing key whose
# fingerprint is pinned below. Nothing is executed or trusted before it is
# verified: the key must match the pin, the checksum list must carry a good
# signature from that key, the archive must match the list, and the profile
# must carry its own good signature.
#
#   install-intend.sh [--version vX.Y.Z] [--home DIR]
#
# Defaults: the latest release; INTEND_HOME=$HOME/.intend, with bin/intend,
# profiles/agent-skills-registry.toml and state/ underneath.
# Needs: curl, tar, gpg, and sha256sum or shasum.
set -eu
REPO="lovon-spec/Intendancy"
KEY_FINGERPRINT="A9B389C058DD177B3303A13522FC08F0A26D3D18"
VERSION=""; HOME_DIR="${INTEND_HOME:-$HOME/.intend}"
while [ $# -gt 0 ]; do case "$1" in
  --version) VERSION="$2"; shift 2;; --home) HOME_DIR="$2"; shift 2;;
  -h|--help) sed -n '2,13p' "$0"; exit 0;; *) echo "unknown argument: $1" >&2; exit 2;; esac; done
for t in curl tar gpg; do command -v "$t" >/dev/null 2>&1 || { echo "missing tool: $t" >&2; exit 2; }; done
if command -v sha256sum >/dev/null 2>&1; then SHA="sha256sum"; elif command -v shasum >/dev/null 2>&1; then SHA="shasum -a 256"; else echo "missing tool: sha256sum or shasum" >&2; exit 2; fi
case "$(uname -s)" in Linux) OS=linux;; Darwin) OS=macos;; *) echo "unsupported OS: $(uname -s)" >&2; exit 2;; esac
case "$(uname -m)" in x86_64|amd64) ARCH=x86_64;; arm64|aarch64) ARCH=aarch64;; *) echo "unsupported architecture: $(uname -m)" >&2; exit 2;; esac
if [ -z "$VERSION" ]; then
  # "latest" excludes pre-releases, which is all there is during the alpha; fall back to the newest release of any kind.
  VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
  [ -n "$VERSION" ] || VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=1" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)
  [ -n "$VERSION" ] || { echo "could not determine the newest release" >&2; exit 1; }
fi
BASE="https://github.com/$REPO/releases/download/$VERSION"; ARCHIVE="intend-${VERSION#v}-$OS-$ARCH.tar.gz"
WORK=$(mktemp -d); GNUPGHOME=$(mktemp -d); export GNUPGHOME; trap 'rm -rf "$WORK" "$GNUPGHOME"' EXIT INT TERM
echo "release $VERSION, asset $ARCHIVE"
for f in RELEASE-SIGNING-KEY.asc SHA256SUMS SHA256SUMS.asc agent-skills-registry.toml agent-skills-registry.toml.asc "$ARCHIVE" "$ARCHIVE.asc"; do
  curl -fsSL "$BASE/$f" -o "$WORK/$f" || { echo "download failed: $f" >&2; exit 1; }; done
gpg --batch --quiet --import "$WORK/RELEASE-SIGNING-KEY.asc"
GOT=$(gpg --batch --with-colons --fingerprint 2>/dev/null | awk -F: '/^fpr/{print $10; exit}')
[ "$GOT" = "$KEY_FINGERPRINT" ] || { echo "signing key fingerprint $GOT does not match the pinned $KEY_FINGERPRINT" >&2; exit 1; }
for f in SHA256SUMS agent-skills-registry.toml "$ARCHIVE"; do
  gpg --batch --quiet --verify "$WORK/$f.asc" "$WORK/$f" 2>/dev/null || { echo "bad or missing signature: $f" >&2; exit 1; }; done
( cd "$WORK" && grep " $ARCHIVE$\| \*$ARCHIVE$\|  $ARCHIVE$" SHA256SUMS | $SHA -c - >/dev/null ) || { echo "checksum mismatch: $ARCHIVE" >&2; exit 1; }
mkdir -p "$HOME_DIR/bin" "$HOME_DIR/profiles" "$HOME_DIR/state"
tar -xzf "$WORK/$ARCHIVE" -C "$WORK"; BIN=$(find "$WORK" -type f -name intend | head -1); [ -n "$BIN" ] || { echo "archive holds no intend binary" >&2; exit 1; }
install -m 0755 "$BIN" "$HOME_DIR/bin/intend"; install -m 0644 "$WORK/agent-skills-registry.toml" "$HOME_DIR/profiles/agent-skills-registry.toml"
echo "installed $HOME_DIR/bin/intend ($("$HOME_DIR/bin/intend" --version)) and $HOME_DIR/profiles/agent-skills-registry.toml, both verified against key $KEY_FINGERPRINT"
echo "next: $HOME_DIR/bin/intend --profile $HOME_DIR/profiles/agent-skills-registry.toml --state-dir $HOME_DIR/state update"
