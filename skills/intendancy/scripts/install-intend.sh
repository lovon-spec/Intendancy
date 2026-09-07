#!/usr/bin/env sh
# Install the intend CLI and the Agent Skills Registry profile from one fixed,
# signed GitHub release. This skill is content-addressed and immutable, so the
# release it installs is fixed too: the archive for each platform and the
# profile are identified below by SHA-256 digest, and only those exact bytes
# are ever installed. A newer CLI ships as a new version of this skill.
#
# Before anything is placed or executed, all of the following must hold:
#   1. the downloaded key file holds exactly one primary key, whose fingerprint
#      equals KEY_FINGERPRINT;
#   2. the signed checksum list, the archive and the profile each carry a
#      signature that GnuPG reports as valid AND made under that primary key
#      (VALIDSIG binding, so a bundled second key cannot vouch for anything);
#   3. the archive's digest equals its entry in the allowlist and in the
#      signed checksum list; the profile's digest equals its allowlist entry.
# Any failure leaves nothing installed.
#
#   install-intend.sh [--home DIR]
#
# INTEND_HOME (or --home) defaults to $HOME/.intend, with bin/intend,
# profiles/agent-skills-registry.toml and state/ underneath.
# INTEND_RELEASE_BASE_URL may point at a mirror; the pins still decide.
# Needs: curl, tar, gpg (2.x), and sha256sum or shasum.
set -eu
VERSION="v0.1.0-alpha.1"
REPO="lovon-spec/Intendancy"
KEY_FINGERPRINT="A9B389C058DD177B3303A13522FC08F0A26D3D18"
DIGEST_LINUX_X86_64="526b98590b1432ee9e8f7cce6ffdf7de31bee28fc49df9f521b107ff4e969546"
DIGEST_LINUX_AARCH64="d82b3ce3700fac14b116c8d21eca6da6ef5ed8470e5d30475c480b5403d4b1e8"
DIGEST_MACOS_X86_64="b5ff8937c34ffa15822606dfa814ac65a18b76fc2ba909664d3655d35ff500ac"
DIGEST_MACOS_AARCH64="602d872e3b6e884bf03037ce6984da8b96ad3cd9d029a4e5502c8d0a6903a721"
DIGEST_PROFILE="805c24364d9caa4994601216decb78c6ef8793cf0bb2a254ce43eb7abb4e9bfb"
HOME_DIR="${INTEND_HOME:-$HOME/.intend}"
while [ $# -gt 0 ]; do case "$1" in
  --home) HOME_DIR="$2"; shift 2;;
  -h|--help) sed -n '2,24p' "$0"; exit 0;;
  *) echo "unknown argument: $1" >&2; exit 2;; esac; done
for t in curl tar gpg; do command -v "$t" >/dev/null 2>&1 || { echo "missing tool: $t" >&2; exit 2; }; done
if command -v sha256sum >/dev/null 2>&1; then digest_of() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then digest_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
else echo "missing tool: sha256sum or shasum" >&2; exit 2; fi
case "$(uname -s)" in Linux) OS=linux;; Darwin) OS=macos;; *) echo "unsupported OS: $(uname -s)" >&2; exit 2;; esac
case "$(uname -m)" in x86_64|amd64) ARCH=x86_64;; arm64|aarch64) ARCH=aarch64;; *) echo "unsupported architecture: $(uname -m)" >&2; exit 2;; esac
case "$OS-$ARCH" in
  linux-x86_64) EXPECTED="$DIGEST_LINUX_X86_64";; linux-aarch64) EXPECTED="$DIGEST_LINUX_AARCH64";;
  macos-x86_64) EXPECTED="$DIGEST_MACOS_X86_64";; macos-aarch64) EXPECTED="$DIGEST_MACOS_AARCH64";; esac
BASE="${INTEND_RELEASE_BASE_URL:-https://github.com/$REPO/releases/download/$VERSION}"
ARCHIVE="intend-${VERSION#v}-$OS-$ARCH.tar.gz"
WORK=$(mktemp -d); GNUPGHOME=$(mktemp -d); export GNUPGHOME
trap 'rm -rf "$WORK" "$GNUPGHOME"' EXIT INT TERM
echo "release $VERSION, asset $ARCHIVE"
for f in RELEASE-SIGNING-KEY.asc SHA256SUMS SHA256SUMS.asc agent-skills-registry.toml agent-skills-registry.toml.asc "$ARCHIVE" "$ARCHIVE.asc"; do
  curl -fsSL "$BASE/$f" -o "$WORK/$f" || { echo "download failed: $f" >&2; exit 1; }
done
# 1. Exactly one primary key, and it is the pinned one.
gpg --batch --quiet --import "$WORK/RELEASE-SIGNING-KEY.asc" 2>/dev/null || { echo "the key file did not import" >&2; exit 1; }
PRIMARIES=$(gpg --batch --with-colons --list-keys 2>/dev/null | grep -c '^pub:' || true)
[ "$PRIMARIES" = "1" ] || { echo "the key file holds $PRIMARIES primary keys; exactly one is allowed" >&2; exit 1; }
GOT=$(gpg --batch --with-colons --fingerprint 2>/dev/null | awk -F: '/^fpr:/{print $10; exit}')
[ "$GOT" = "$KEY_FINGERPRINT" ] || { echo "signing key fingerprint $GOT does not match the pinned $KEY_FINGERPRINT" >&2; exit 1; }
# 2. A signature counts only if GnuPG reports it valid under the pinned primary key.
verify_bound() {
  status=$(gpg --batch --status-fd 1 --verify "$1.asc" "$1" 2>/dev/null) || { echo "bad or missing signature: $(basename "$1")" >&2; exit 1; }
  primary=$(printf '%s\n' "$status" | awk '/^\[GNUPG:\] VALIDSIG /{print $12; exit}')
  [ "$primary" = "$KEY_FINGERPRINT" ] || { echo "signature on $(basename "$1") is not under the pinned key (got '${primary:-none}')" >&2; exit 1; }
}
verify_bound "$WORK/SHA256SUMS"; verify_bound "$WORK/$ARCHIVE"; verify_bound "$WORK/agent-skills-registry.toml"
# 3. Digests: allowlist and signed list must both match.
ACTUAL=$(digest_of "$WORK/$ARCHIVE")
[ "$ACTUAL" = "$EXPECTED" ] || { echo "archive digest $ACTUAL is not the pinned $EXPECTED" >&2; exit 1; }
LISTED=$(awk -v f="$ARCHIVE" '$2 == f || $2 == "*" f {print $1; exit}' "$WORK/SHA256SUMS")
[ "$LISTED" = "$EXPECTED" ] || { echo "the signed checksum list says $LISTED for $ARCHIVE, pinned $EXPECTED" >&2; exit 1; }
PACTUAL=$(digest_of "$WORK/agent-skills-registry.toml")
[ "$PACTUAL" = "$DIGEST_PROFILE" ] || { echo "profile digest $PACTUAL is not the pinned $DIGEST_PROFILE" >&2; exit 1; }
# Install.
mkdir -p "$HOME_DIR/bin" "$HOME_DIR/profiles" "$HOME_DIR/state"
tar -xzf "$WORK/$ARCHIVE" -C "$WORK"
BIN=$(find "$WORK" -type f -name intend | head -1); [ -n "$BIN" ] || { echo "archive holds no intend binary" >&2; exit 1; }
install -m 0755 "$BIN" "$HOME_DIR/bin/intend"
install -m 0644 "$WORK/agent-skills-registry.toml" "$HOME_DIR/profiles/agent-skills-registry.toml"
echo "installed $HOME_DIR/bin/intend ($("$HOME_DIR/bin/intend" --version)) and $HOME_DIR/profiles/agent-skills-registry.toml"
echo "verified: key $KEY_FINGERPRINT, archive sha256:$EXPECTED, profile sha256:$DIGEST_PROFILE"
