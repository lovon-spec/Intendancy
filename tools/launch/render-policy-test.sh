#!/usr/bin/env bash
# Preflight regression for render-policy.sh: a remote renderer must bypass the local-Chrome check, and the
# local mode must still refuse without a usable Chrome. Uses an unreachable host so no render happens.
set -u
HERE=$(cd "$(dirname "$0")" && pwd); OUT=$(mktemp -d)/out.pdf; pass=0; fail=0
# PATH is cut down to the system directories so the script cannot discover a Chrome of its own; the
# CHROME value under test is always a path that does not exist.
run() { PATH=/usr/bin:/bin CHROME="$1" RENDER_SSH="$2" "$HERE/render-policy.sh" "$OUT" >/dev/null 2>"$OUT.err"; echo $?; }
check() { if [ "$1" = "$2" ]; then echo "ok   $3"; pass=$((pass+1)); else echo "FAIL $3 (got $1, want $2): $(tail -1 "$OUT.err")"; fail=$((fail+1)); fi; }
rc=$(run "/nonexistent/chrome" ""); check "$rc" 2 "local mode without a usable Chrome refuses at preflight"
rc=$(run "/nonexistent/chrome" "nobody@test.invalid"); [ "$rc" != 2 ] && grep -q 'remote mktemp failed' "$OUT.err"; check "$?" 0 "remote mode without a local Chrome passes preflight and fails at ssh"
echo "$pass passed, $fail failed"; [ "$fail" = 0 ]
