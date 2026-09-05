#!/usr/bin/env bash
# Prove that a pinned CID is retrievable through public trustless gateways: fetch
# the complete DAG as a CAR from each gateway and verify it locally with
# verify-car.mjs (hashes, CID forms, completeness, root identity).
#
#   check-gateways.sh <cid> [gateway-origin ...]
#
# Defaults to the gateways the evidence display and the CLI profile use.
set -uo pipefail
CID=${1:?usage: check-gateways.sh <cid> [gateway-origin ...]}; shift
GATEWAYS=("$@"); [ ${#GATEWAYS[@]} -gt 0 ] || GATEWAYS=(https://dweb.link https://ipfs.io https://trustless-gateway.link)
HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
status=0
for gw in "${GATEWAYS[@]}"; do
  out="$TMP/$(echo "$gw" | tr -c 'A-Za-z0-9' '_').car"
  code=$(curl -sS -L --max-time 120 -H 'Accept: application/vnd.ipld.car' -o "$out" -w '%{http_code}' "$gw/ipfs/$CID?format=car&dag-scope=all" 2>"$TMP/err") || code="curl: $(tr -d '\n' < "$TMP/err")"
  if [ "$code" != "200" ]; then
    echo "FAIL $gw  HTTP $code"; status=1; continue
  fi
  if result=$("$HERE/verify-car.sh" "$out" "$CID" 2>&1 | tail -1); then
    echo "OK   $gw  $result"
  else
    echo "FAIL $gw  $result"; status=1
  fi
done
exit $status
