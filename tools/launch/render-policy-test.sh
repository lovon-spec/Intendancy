#!/usr/bin/env bash
# Hermetic tests for render-policy.sh's control flow: ssh and scp are replaced by fixtures in a private PATH,
# no browser and no network are needed. The fixtures emulate the remote side in a local directory.
set -u
HERE=$(cd "$(dirname "$0")" && pwd); WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
BIN="$WORK/bin"; mkdir -p "$BIN"; PY=$(dirname "$(command -v python3)")
cat > "$BIN/ssh" <<'FAKE'
#!/bin/sh
cmd="$2"
case "$cmd" in
  *mktemp*) [ "${FAKE_MKTEMP_FAIL:-0}" = 1 ] && exit 255; echo "$FAKE_REMOTE";;
  *"rm -rf"*) exit "${FAKE_RM_EXIT:-0}";;
  *print-to-pdf*) [ "${FAKE_RENDER_FAIL:-0}" = 1 ] && exit 1; printf '%%PDF-1.4 fixture' > "$FAKE_REMOTE/policy.pdf";;
  *) exit 1;;
esac
FAKE
cat > "$BIN/scp" <<'FAKE'
#!/bin/sh
shift $(( $# - 2 )); src=${1#*:}; dst=${2#*:}; cp "$src" "$dst"
FAKE
chmod +x "$BIN/ssh" "$BIN/scp"
pass=0; fail=0; n=0
# run <RENDER_SSH> [VAR=value ...]: runs the script in the parent shell and leaves RC and OUT set.
run() { n=$((n+1)); OUT="$WORK/out$n.pdf"; export FAKE_REMOTE="$WORK/remote$n"; mkdir -p "$FAKE_REMOTE"; remote=$1; shift
  env "$@" PATH="$BIN:$PY:/usr/bin:/bin" CHROME="/nonexistent/chrome" RENDER_SSH="$remote" "$HERE/render-policy.sh" "$OUT" >/dev/null 2>"$OUT.err"; RC=$?; }
check() { if [ "$1" = 0 ]; then echo "ok   $2"; pass=$((pass+1)); else echo "FAIL $2 (exit $RC): $(tail -1 "$OUT.err")"; fail=$((fail+1)); fi; }
run ""; [ "$RC" = 2 ]; check $? "local mode without a usable Chrome refuses at preflight"
run "fixture@remote" FAKE_MKTEMP_FAIL=1; [ "$RC" = 1 ] && grep -q 'remote mktemp failed' "$OUT.err"; check $? "remote mode passes preflight without a local Chrome and fails at ssh"
run "fixture@remote"; [ "$RC" = 0 ] && head -c 5 "$OUT" | grep -q '%PDF-'; check $? "remote render succeeds and the PDF comes back"
run "fixture@remote" FAKE_RENDER_FAIL=1; [ "$RC" = 1 ] && [ ! -s "$OUT" ] && grep -q 'remote renderer failed' "$OUT.err"; check $? "a failing remote renderer is a failure with no PDF accepted"
run "fixture@remote" FAKE_RM_EXIT=74; [ "$RC" = 0 ] && head -c 5 "$OUT" | grep -q '%PDF-' && grep -q 'was not removed' "$OUT.err"; check $? "a failing remote cleanup keeps the result and warns"
echo "$pass passed, $fail failed"; [ "$fail" = 0 ]
