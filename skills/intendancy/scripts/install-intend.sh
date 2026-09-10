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
#      signed checksum list; the profile's digest equals its allowlist entry;
#   4. the extracted intend executable equals its pinned digest, the one the
#      skill's description discloses.
# Any failure leaves nothing installed.
#
#   install-intend.sh [--home DIR]
#
# INTEND_HOME (or --home) defaults to $HOME/.intend, with bin/intend,
# profiles/agent-skills-registry.toml and state/ underneath.
# INTEND_RELEASE_BASE_URL may point at a mirror; the pins still decide.
# Needs: curl, tar, gpg (2.x), and sha256sum or shasum.
set -eu
VERSION="v0.1.0-alpha.2"
REPO="lovon-spec/Intendancy"
KEY_FINGERPRINT="A9B389C058DD177B3303A13522FC08F0A26D3D18"
DIGEST_LINUX_X86_64="694f3948bef7aff73db50ddb580a0b1e59efe7d7517b8c96a16dfabd2b5add35"
DIGEST_LINUX_AARCH64="5188d65156713fec47edfc019b9d8e3b1d5346c721534ece0551983bcf178f98"
DIGEST_MACOS_X86_64="dea10ba3ebe0a55049af658e09937d42dbcab4985355e7e556d0304ee9f1dcbd"
DIGEST_MACOS_AARCH64="3f48b0a61e7da36e178d94dd9119bcfe863f31132794b8e621b862b1061277fc"
BINARY_LINUX_X86_64="0f6d6eccd27b1343ae27767cdbfeb6fa2c674f99462ecd84e1b3844ba0975f20"
BINARY_LINUX_AARCH64="70713479af1ddd070f27e9c931f2c7e21d520c82fcf4bd0dba10932533ac916f"
BINARY_MACOS_X86_64="5230ebc9b190cc3a35057619a73501e03a4bb91e4cd6c05752d133624392679b"
BINARY_MACOS_AARCH64="48257332ee85fbcff417c881c4634dc82bd5bd8791f46ec085b9e5fa6b1d2cbd"
DIGEST_PROFILE="0d6c95d328e0d7de730be7cc280dc4ff05af5ff80c36c5c0659f48352a5c0411"
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
  linux-x86_64) EXPECTED="$DIGEST_LINUX_X86_64"; EXPECTED_BIN="$BINARY_LINUX_X86_64";; linux-aarch64) EXPECTED="$DIGEST_LINUX_AARCH64"; EXPECTED_BIN="$BINARY_LINUX_AARCH64";;
  macos-x86_64) EXPECTED="$DIGEST_MACOS_X86_64"; EXPECTED_BIN="$BINARY_MACOS_X86_64";; macos-aarch64) EXPECTED="$DIGEST_MACOS_AARCH64"; EXPECTED_BIN="$BINARY_MACOS_AARCH64";; esac
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
# 4. The extracted executable itself must match its pinned digest before it is installed or run.
BACTUAL=$(digest_of "$BIN")
[ "$BACTUAL" = "$EXPECTED_BIN" ] || { echo "binary digest $BACTUAL is not the pinned $EXPECTED_BIN" >&2; exit 1; }
install -m 0755 "$BIN" "$HOME_DIR/bin/intend"
install -m 0644 "$WORK/agent-skills-registry.toml" "$HOME_DIR/profiles/agent-skills-registry.toml"
echo "installed $HOME_DIR/bin/intend ($("$HOME_DIR/bin/intend" --version)) and $HOME_DIR/profiles/agent-skills-registry.toml"
echo "verified: key $KEY_FINGERPRINT, archive sha256:$EXPECTED, binary sha256:$EXPECTED_BIN, profile sha256:$DIGEST_PROFILE"
