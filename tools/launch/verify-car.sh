#!/usr/bin/env bash
# Verify a CAR file (CID forms, block hashes, DAG completeness, root identity).
#   verify-car.sh <file.car> [expected-root-cid]
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
exec node "$HERE/../../frontend/tools/verify-car.mjs" "$@"
