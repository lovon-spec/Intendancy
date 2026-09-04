#!/usr/bin/env bash
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"

legacy_pattern='intend[h]ub'
if git grep -Iin -E "$legacy_pattern" -- ':!docs/BRAND.md'; then
  echo 'Deprecated product name remains outside docs/BRAND.md.' >&2
  exit 1
fi
if git ls-files | grep -i -E "$legacy_pattern"; then
  echo 'A tracked path still contains the deprecated product name.' >&2
  exit 1
fi

grep -Fq '"name": "intendancy-frontend"' frontend/package.json
grep -Fq '<title>Intendancy — Verifiable Agent Skill Registry</title>' frontend/index.html
grep -Fq '<meta name="theme-color" content="#11111b" />' frontend/index.html
grep -Fq 'Intendancy' frontend/src/components/layout/Header.tsx
grep -Fq '#89b4fa' assets/intendancy-mark.svg
grep -Fq '#f9e2af' assets/intendancy-mark.svg
grep -Fq '#a6e3a1' assets/intendancy-mark.svg
cmp docs/listing-policy.md frontend/public/listing-policy.md
cmp assets/intendancy-mark.svg frontend/public/favicon.svg
cmp assets/intendancy-mark.svg meta-evidence/intendancy-logo.svg
git diff --check
