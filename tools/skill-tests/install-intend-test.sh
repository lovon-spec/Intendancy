#!/usr/bin/env bash
# Regression tests for skills/intendancy/scripts/install-intend.sh: the
# installer must install from the genuine release and refuse a wrong key pin,
# an unrelated signing key, a bundle of the genuine key with an unrelated key
# whose signatures are the ones on the files (the mixed-key bypass), and a
# corrupted archive. Assets are served from a local HTTP server; the genuine
# assets are downloaded once from the GitHub release.
#
#   tools/skill-tests/install-intend-test.sh [assets-dir]
set -euo pipefail
set +m
ROOT=$(cd "$(dirname "$0")/../.." && pwd); INSTALLER="$ROOT/skills/intendancy/scripts/install-intend.sh"
VERSION=$(sed -n 's/^VERSION="\(.*\)"$/\1/p' "$INSTALLER")
ASSETS=${1:-"$ROOT/tools/skill-tests/.assets-$VERSION"}
case "$(uname -s)-$(uname -m)" in Linux-x86_64|Linux-amd64) A="linux-x86_64";; Linux-aarch64|Linux-arm64) A="linux-aarch64";; Darwin-x86_64) A="macos-x86_64";; Darwin-arm64) A="macos-aarch64";; *) echo "unsupported platform" >&2; exit 2;; esac
ARCHIVE="intend-${VERSION#v}-$A.tar.gz"
if [ ! -f "$ASSETS/$ARCHIVE" ]; then mkdir -p "$ASSETS"; ( cd "$ASSETS" && gh release download "$VERSION" --repo lovon-spec/Intendancy -p 'RELEASE-SIGNING-KEY.asc' -p 'SHA256SUMS*' -p 'agent-skills-registry.toml*' -p "$ARCHIVE*" --clobber ); fi
SERVER=; TMP=$(mktemp -d); trap 'stop; rm -rf "$TMP"' EXIT
# An unrelated key, generated for the test.
export GNUPGHOME="$TMP/gnupg"; mkdir -m 700 "$GNUPGHOME"
gpg --batch --quiet --passphrase '' --quick-gen-key 'Unrelated Test Key <test@example.invalid>' ed25519 sign 0 2>/dev/null
gpg --batch --armor --export > "$TMP/unrelated.asc"
sign_with_unrelated() { for f in SHA256SUMS agent-skills-registry.toml "$ARCHIVE"; do gpg --batch --yes --quiet --passphrase '' --detach-sign --armor -o "$1/$f.asc" "$1/$f" 2>/dev/null; done; }
fixture() { d="$TMP/fx-$1"; mkdir -p "$d"; find "$ASSETS" -maxdepth 1 -type f -exec cp {} "$d/" \; ; echo "$d"; }
serve() { PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'); python3 -u -m http.server --bind 127.0.0.1 --directory "$1" "$PORT" >"$TMP/server.log" 2>&1 & SERVER=$!; for i in $(seq 1 50); do curl -fsS -o /dev/null "http://127.0.0.1:$PORT/SHA256SUMS" 2>/dev/null && break; sleep 0.1; done; URL="http://127.0.0.1:$PORT"; }
stop() { if [ -n "$SERVER" ]; then kill "$SERVER" 2>/dev/null; wait "$SERVER" 2>/dev/null || true; fi; SERVER=; }
run() { home="$TMP/home-$1"; shift; INTEND_RELEASE_BASE_URL="$1" INTEND_HOME="$home" sh "${2:-$INSTALLER}" >"$TMP/out" 2>&1 && rc=0 || rc=$?; if [ -x "$home/bin/intend" ]; then installed=yes; else installed=no; fi; }
pass=0; fail=0; check() { if [ "$1" = "$2" ] && [ "$3" = "$4" ]; then echo "ok   $5"; pass=$((pass+1)); else echo "FAIL $5 (rc=$1 want $2, installed=$3 want $4): $(tail -1 "$TMP/out")"; fail=$((fail+1)); fi; }
# 1. Genuine release.
d=$(fixture genuine); serve "$d"; run genuine "$URL"; check "$rc" 0 "$installed" yes "genuine release installs"; stop
# 2. Wrong pin.
sed 's/^KEY_FINGERPRINT=.*/KEY_FINGERPRINT="0000000000000000000000000000000000000000"/' "$INSTALLER" > "$TMP/wrongpin.sh"; serve "$d"; run wrongpin "$URL" "$TMP/wrongpin.sh"; check "$rc" 1 "$installed" no "wrong key pin refuses"; stop
# 3. Unrelated key only, files signed by it.
d=$(fixture unrelated); cp "$TMP/unrelated.asc" "$d/RELEASE-SIGNING-KEY.asc"; sign_with_unrelated "$d"; serve "$d"; run unrelated "$URL"; check "$rc" 1 "$installed" no "unrelated key refuses"; stop
# 4. Mixed bundle: genuine key + unrelated key in the key file, signatures by the unrelated key.
d=$(fixture mixed); cat "$ASSETS/RELEASE-SIGNING-KEY.asc" "$TMP/unrelated.asc" > "$d/RELEASE-SIGNING-KEY.asc"; sign_with_unrelated "$d"; serve "$d"; run mixed "$URL"; check "$rc" 1 "$installed" no "mixed key bundle refuses"; stop
# 5. Corrupted archive with genuine signatures.
d=$(fixture corrupt); printf 'x' >> "$d/$ARCHIVE"; serve "$d"; run corrupt "$URL"; check "$rc" 1 "$installed" no "corrupted archive refuses"; stop
echo "$pass passed, $fail failed"; [ "$fail" = 0 ]
