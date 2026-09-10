#!/usr/bin/env sh
# Install the curate-export binary from one fixed, signed GitHub release of
# lovon-spec/Intendancy. The digests below are PENDING_RELEASE placeholders:
# they are filled, and this skill is submitted, at the first release that ships
# curate-export. Until then the script refuses to run.
#
# Before anything is placed or executed: the downloaded key file holds exactly
# one primary key with the pinned fingerprint; the signed checksum list and the
# archive carry signatures made under that key (VALIDSIG binding); the archive
# and the extracted executable match their pinned digests. Any failure leaves
# nothing installed.
#
#   install-curate-export.sh [--home DIR]
#
# INTEND_HOME (or --home) defaults to $HOME/.intend, with bin/curate-export
# underneath. INTEND_RELEASE_BASE_URL may point at a mirror; the pins decide.
# Needs: curl, tar, gpg (2.x), and sha256sum or shasum.
set -eu
VERSION="PENDING_RELEASE"
REPO="lovon-spec/Intendancy"
KEY_FINGERPRINT="A9B389C058DD177B3303A13522FC08F0A26D3D18"
DIGEST_LINUX_X86_64="PENDING_RELEASE"
DIGEST_LINUX_AARCH64="PENDING_RELEASE"
DIGEST_MACOS_X86_64="PENDING_RELEASE"
DIGEST_MACOS_AARCH64="PENDING_RELEASE"
BINARY_LINUX_X86_64="PENDING_RELEASE"
BINARY_LINUX_AARCH64="PENDING_RELEASE"
BINARY_MACOS_X86_64="PENDING_RELEASE"
BINARY_MACOS_AARCH64="PENDING_RELEASE"
HOME_DIR="${INTEND_HOME:-$HOME/.intend}"
while [ $# -gt 0 ]; do case "$1" in
  --home) HOME_DIR="$2"; shift 2;;
  -h|--help) sed -n '2,17p' "$0"; exit 0;;
  *) echo "unknown argument: $1" >&2; exit 2;; esac; done
case "$VERSION$DIGEST_LINUX_X86_64$BINARY_LINUX_X86_64" in *PENDING_RELEASE*) echo "this skill version predates the release that ships curate-export; its pins are placeholders" >&2; exit 2;; esac
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
ARCHIVE="curate-export-${VERSION#v}-$OS-$ARCH.tar.gz"
WORK=$(mktemp -d); GNUPGHOME=$(mktemp -d); export GNUPGHOME
trap 'rm -rf "$WORK" "$GNUPGHOME"' EXIT INT TERM
echo "release $VERSION, asset $ARCHIVE"
for f in RELEASE-SIGNING-KEY.asc SHA256SUMS SHA256SUMS.asc "$ARCHIVE" "$ARCHIVE.asc"; do
  curl -fsSL "$BASE/$f" -o "$WORK/$f" || { echo "download failed: $f" >&2; exit 1; }
done
gpg --batch --quiet --import "$WORK/RELEASE-SIGNING-KEY.asc" 2>/dev/null || { echo "the key file did not import" >&2; exit 1; }
PRIMARIES=$(gpg --batch --with-colons --list-keys 2>/dev/null | grep -c '^pub:' || true)
[ "$PRIMARIES" = "1" ] || { echo "the key file holds $PRIMARIES primary keys; exactly one is allowed" >&2; exit 1; }
GOT=$(gpg --batch --with-colons --fingerprint 2>/dev/null | awk -F: '/^fpr:/{print $10; exit}')
[ "$GOT" = "$KEY_FINGERPRINT" ] || { echo "signing key fingerprint $GOT does not match the pinned $KEY_FINGERPRINT" >&2; exit 1; }
verify_bound() {
  status=$(gpg --batch --status-fd 1 --verify "$1.asc" "$1" 2>/dev/null) || { echo "bad or missing signature: $(basename "$1")" >&2; exit 1; }
  primary=$(printf '%s\n' "$status" | awk '/^\[GNUPG:\] VALIDSIG /{print $12; exit}')
  [ "$primary" = "$KEY_FINGERPRINT" ] || { echo "signature on $(basename "$1") is not under the pinned key (got '${primary:-none}')" >&2; exit 1; }
}
verify_bound "$WORK/SHA256SUMS"; verify_bound "$WORK/$ARCHIVE"
ACTUAL=$(digest_of "$WORK/$ARCHIVE")
[ "$ACTUAL" = "$EXPECTED" ] || { echo "archive digest $ACTUAL is not the pinned $EXPECTED" >&2; exit 1; }
LISTED=$(awk -v f="$ARCHIVE" '$2 == f || $2 == "*" f {print $1; exit}' "$WORK/SHA256SUMS")
[ "$LISTED" = "$EXPECTED" ] || { echo "the signed checksum list says $LISTED for $ARCHIVE, pinned $EXPECTED" >&2; exit 1; }
mkdir -p "$HOME_DIR/bin"
tar -xzf "$WORK/$ARCHIVE" -C "$WORK"
BIN=$(find "$WORK" -type f -name curate-export | head -1); [ -n "$BIN" ] || { echo "archive holds no curate-export binary" >&2; exit 1; }
BACTUAL=$(digest_of "$BIN")
[ "$BACTUAL" = "$EXPECTED_BIN" ] || { echo "binary digest $BACTUAL is not the pinned $EXPECTED_BIN" >&2; exit 1; }
install -m 0755 "$BIN" "$HOME_DIR/bin/curate-export"
echo "installed $HOME_DIR/bin/curate-export ($("$HOME_DIR/bin/curate-export" --version))"
echo "verified: key $KEY_FINGERPRINT, archive sha256:$EXPECTED, binary sha256:$EXPECTED_BIN"
