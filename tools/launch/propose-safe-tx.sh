#!/usr/bin/env bash
# Queue a transaction in a Safe through the Safe Transaction Service, the way a
# registered proposer does it: compute the Safe transaction hash on chain, sign
# that hash with the proposer's key, post the proposal. The Safe's owners then
# confirm and execute it in the Safe app; queued transactions execute in nonce
# order.
#
#   propose-safe-tx.sh <safe> <to> <calldata-file> [--value <wei>] [--nonce <n>]
#                      [--origin <text>] [--key-file <path>] [--rpc <url>]
#
# <calldata-file> holds the 0x-prefixed hex calldata. The operation is always
# Call; safeTxGas, baseGas and gasPrice are 0 (no refund), gasToken and
# refundReceiver are the zero address. The nonce defaults to the Safe's current
# nonce; pass --nonce to queue behind transactions already waiting.
#
# Without --key-file this is a dry run: it prints the Safe transaction hash and
# writes the proposal body without a signature, posting nothing. With
# --key-file it signs and posts; the key is read into the cast invocation only
# and never printed. The sender must be an owner of the Safe, or a proposer
# (transaction-service delegate) that an owner registered in the Safe app.
# The service host is the Safe{Core} API (SAFE_TX_SERVICE overrides it; the
# older per-chain host only redirects there).
set -euo pipefail
usage() { echo "usage: propose-safe-tx.sh <safe> <to> <calldata-file> [--value <wei>] [--nonce <n>] [--origin <text>] [--key-file <path>] [--rpc <url>]" >&2; exit 2; }
SAFE=${1:-}; TO=${2:-}; CALLDATA_FILE=${3:-}
[ -n "$SAFE" ] && [ -n "$TO" ] && [ -n "$CALLDATA_FILE" ] || usage
shift 3
VALUE=0; NONCE=""; ORIGIN=""; KEY_FILE=""
RPC=${GNOSIS_RPC_URL:-https://rpc.gnosischain.com}
SERVICE=${SAFE_TX_SERVICE:-https://api.safe.global/tx-service/gno}
while [ $# -gt 0 ]; do
  case "$1" in
    --value) VALUE=${2:?}; shift 2;;
    --nonce) NONCE=${2:?}; shift 2;;
    --origin) ORIGIN=${2:?}; shift 2;;
    --key-file) KEY_FILE=${2:?}; shift 2;;
    --rpc) RPC=${2:?}; shift 2;;
    *) usage;;
  esac
done
command -v cast >/dev/null || { echo "foundry (cast) is required" >&2; exit 2; }
[ -r "$CALLDATA_FILE" ] || { echo "cannot read $CALLDATA_FILE" >&2; exit 2; }
DATA=$(tr -d '[:space:]' < "$CALLDATA_FILE")
case "$DATA" in 0x[0-9a-fA-F]*) ;; *) echo "the calldata file must hold 0x-prefixed hex" >&2; exit 2;; esac
ZERO=0x0000000000000000000000000000000000000000
[ -n "$NONCE" ] || NONCE=$(cast call "$SAFE" 'nonce()(uint256)' --rpc-url "$RPC")
HASH=$(cast call "$SAFE" 'getTransactionHash(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,uint256)(bytes32)' \
  "$TO" "$VALUE" "$DATA" 0 0 0 0 "$ZERO" "$ZERO" "$NONCE" --rpc-url "$RPC")
echo "safe:           $SAFE"
echo "to:             $TO"
echo "value (wei):    $VALUE"
echo "calldata:       $(( (${#DATA} - 2) / 2 )) bytes, keccak $(cast keccak "$DATA")"
echo "nonce:          $NONCE"
echo "safeTxHash:     $HASH"
SENDER=""; SIG=""
if [ -n "$KEY_FILE" ]; then
  [ -r "$KEY_FILE" ] || { echo "cannot read the key file" >&2; exit 2; }
  SENDER=$(cast wallet address --private-key "$(tr -d '[:space:]' < "$KEY_FILE")")
  SIG=$(cast wallet sign --no-hash "$HASH" --private-key "$(tr -d '[:space:]' < "$KEY_FILE")")
fi
BODY=$(mktemp "${TMPDIR:-/tmp}/safe-proposal.XXXXXX")
python3 - "$BODY" "$TO" "$VALUE" "$DATA" "$NONCE" "$HASH" "$SENDER" "$SIG" "$ORIGIN" <<'PY'
import json, sys
path, to, value, data, nonce, h, sender, sig, origin = sys.argv[1:10]
body = {
    "to": to, "value": str(int(value)), "data": data, "operation": 0,
    "gasToken": "0x0000000000000000000000000000000000000000",
    "safeTxGas": 0, "baseGas": 0, "gasPrice": "0",
    "refundReceiver": "0x0000000000000000000000000000000000000000",
    "nonce": int(nonce), "contractTransactionHash": h,
}
if sender:
    body["sender"] = sender
    body["signature"] = sig
if origin:
    body["origin"] = origin
with open(path, "w") as f:
    json.dump(body, f)
PY
echo "proposal body:  $BODY"
if [ -z "$KEY_FILE" ]; then
  echo "dry run: no --key-file, nothing posted"
  exit 0
fi
CODE=$(curl -sS -o "$BODY.response" -w '%{http_code}' -X POST -H 'Content-Type: application/json' --data @"$BODY" "$SERVICE/api/v1/safes/$SAFE/multisig-transactions/")
[ "$CODE" = "201" ] || { echo "the transaction service answered HTTP $CODE:" >&2; cat "$BODY.response" >&2; echo >&2; exit 1; }
echo "posted:         HTTP 201 as $SENDER"
curl -sS "$SERVICE/api/v1/multisig-transactions/$HASH/" | python3 -c '
import json, subprocess, sys
t = json.load(sys.stdin)
k = subprocess.run(["cast", "keccak", t["data"]], capture_output=True, text=True).stdout.strip()
print("queued:         nonce", t["nonce"], "to", t["to"], "data keccak", k)
print("proposed by:    delegate", t.get("proposedByDelegate") or "-", "on behalf of", t.get("proposer"))
print("confirmations:  %d of %s, executed %s" % (len(t.get("confirmations") or []), t.get("confirmationsRequired"), t["isExecuted"]))'
