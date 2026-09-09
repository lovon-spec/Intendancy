#!/usr/bin/env bash
# Render docs/listing-policy.md to the PDF that the MetaEvidence fileURI points
# at (/ipfs/<cid>/listing-policy.pdf). Markdown to HTML with python-markdown,
# HTML to PDF with headless Chrome. Chrome stamps a creation date, so the PDF is
# not byte-reproducible: render it once on deployment day, after the
# placeholders are filled, keep the file, and pin that exact file.
#
#   render-policy.sh <out.pdf>
set -euo pipefail
OUT=${1:?usage: render-policy.sh <out.pdf>}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SRC="$ROOT/docs/listing-policy.md"
cmp -s "$SRC" "$ROOT/frontend/public/listing-policy.md" || { echo "the two policy copies differ; fix that first" >&2; exit 1; }
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
[ -x "$CHROME" ] || CHROME=$(command -v google-chrome || command -v chromium || command -v chromium-browser || true)
if [ -z "${RENDER_SSH:-}" ] && { [ -z "$CHROME" ] || [ ! -x "$CHROME" ]; }; then
  echo "headless Chrome not found; set CHROME=/path/to/chrome or RENDER_SSH=user@host" >&2; exit 2
fi
TMP=$(mktemp -d); R=
# Cleanup keeps the script's own exit status: the remote scratch directory is removed on a best-effort basis
# with a warning, the local one always.
cleanup() { rc=$?
  if [ -n "$R" ]; then ssh "$RENDER_SSH" "rm -rf \"$R\"" >/dev/null 2>&1 || echo "warning: remote scratch directory $R on $RENDER_SSH was not removed" >&2; fi
  rm -rf "$TMP"; exit $rc; }
trap cleanup EXIT
python3 - "$SRC" "$TMP/policy.html" <<'PY'
import sys, markdown
src = open(sys.argv[1], encoding="utf-8").read()
body = markdown.markdown(src, extensions=["tables", "fenced_code", "nl2br"])
css = """
body { font: 11pt/1.45 -apple-system, "Helvetica Neue", Helvetica, Arial, sans-serif; color: #111; max-width: 46em; margin: 0 auto; }
h1 { font-size: 18pt; margin: 0 0 0.6em; } h2 { font-size: 14pt; margin-top: 1.6em; } h3 { font-size: 12pt; margin-top: 1.2em; }
table { border-collapse: collapse; font-size: 9.5pt; margin: 0.6em 0; } th, td { border: 1px solid #999; padding: 4px 6px; vertical-align: top; text-align: left; }
code { font: 9.5pt Menlo, Consolas, monospace; background: #f2f2f2; padding: 0 2px; } pre code { display: block; padding: 6px; white-space: pre-wrap; }
li { margin: 0.2em 0; } @page { size: A4; margin: 18mm 16mm; }
"""
html = f'<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Agent Skills Registry — Listing Policy</title><style>{css}</style></head><body>{body}</body></html>'
open(sys.argv[2], "w", encoding="utf-8").write(html)
PY
if [ -n "${RENDER_SSH:-}" ]; then
  # Render on another machine over SSH (RENDER_SSH=user@host, RENDER_CHROME=chromium by default): the HTML
  # goes to a fresh remote temporary directory, Chromium must succeed and produce the PDF, the PDF comes
  # back, and the remote directory is removed whatever happens. Fonts follow that machine, so note the
  # renderer with the pinned file.
  R=$(ssh "$RENDER_SSH" 'mktemp -d "${TMPDIR:-/tmp}/render-policy.XXXXXX"') || { echo "remote mktemp failed" >&2; exit 1; }
  scp -q "$TMP/policy.html" "$RENDER_SSH:$R/policy.html" || { echo "remote copy failed" >&2; exit 1; }
  ssh "$RENDER_SSH" "cd \"$R\" && ${RENDER_CHROME:-chromium} --headless=new --disable-gpu --no-sandbox --no-first-run --user-data-dir=profile --no-pdf-header-footer --print-to-pdf=policy.pdf \"file://$R/policy.html\" >/dev/null 2>&1 && test -s policy.pdf" || { echo "remote renderer failed" >&2; exit 1; }
  scp -q "$RENDER_SSH:$R/policy.pdf" "$OUT" || { echo "remote copy back failed" >&2; exit 1; }
else
  # A private profile directory: the default one can be locked by a running Chrome, which crashes headless mode.
  "$CHROME" --headless=new --disable-gpu --no-sandbox --no-first-run --no-default-browser-check --user-data-dir="$TMP/profile" --no-pdf-header-footer --print-to-pdf="$OUT" "file://$TMP/policy.html" >/dev/null 2>&1
fi
head -c 5 "$OUT" | grep -q '%PDF-' || { echo "Chrome did not produce a PDF" >&2; exit 1; }
echo "$OUT $(wc -c < "$OUT" | tr -d ' ') bytes"
