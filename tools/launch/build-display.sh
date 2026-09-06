#!/usr/bin/env bash
# Build the juror evidence display for production (default public gateways,
# relative asset paths) and produce its directory CID and CAR. Run it twice to
# confirm the build is reproducible before pinning.
#
#   build-display.sh <out.car>
set -euo pipefail
OUT=${1:?usage: build-display.sh <out.car>}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
cd "$ROOT/frontend"
rm -rf dist-evidence
env -u VITE_IPFS_GATEWAYS npm run -s build:evidence >/dev/null
"$HERE/car-of.sh" dist-evidence "$OUT"
