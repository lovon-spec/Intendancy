#!/usr/bin/env bash
# Compute the CIDv1 of a file or directory exactly as `ipfs add -r --cid-version 1`
# does (raw leaves, 256 KiB chunks, hidden files skipped) and export its complete
# DAG as a CAR file, using a throwaway kubo repository so nothing touches the
# machine's IPFS state.
#
#   car-of.sh [--wrap] <path> <out.car>
#
#   --wrap   wrap a single file in a directory, for URIs of the form
#            /ipfs/<cid>/<filename> (the policy PDF and the logo)
#
# Prints one line: <cid> <out.car> <car-bytes>
set -euo pipefail
WRAP=""
if [ "${1:-}" = "--wrap" ]; then WRAP="-w"; shift; fi
SRC=${1:?usage: car-of.sh [--wrap] <path> <out.car>}
OUT=${2:?usage: car-of.sh [--wrap] <path> <out.car>}
command -v ipfs >/dev/null || { echo "kubo (ipfs) is required" >&2; exit 2; }
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
export IPFS_PATH="$TMP/repo"
ipfs init -e --profile=test >/dev/null 2>&1
CID=$(ipfs add -r --cid-version 1 --raw-leaves -Q $WRAP "$SRC")
ipfs dag export "$CID" > "$OUT" 2>/dev/null
echo "$CID $OUT $(wc -c < "$OUT" | tr -d ' ')"
