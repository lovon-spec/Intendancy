#!/usr/bin/env bash
# Live smoke for `curate-export`: snapshot all four Kleros Scout registries on
# Gnosis against public RPCs and gateways, render the Tokens list as a token
# list with a diff against the discontinued t2crtokens.eth export, and run an
# offline lookup on Address Tags. Prints wall-clock time and RPC calls per
# registry, and checks determinism with a second Tokens export.
#
#   cli/tools/smoke-curate-export.sh [work-dir]
#
# Needs a built release binary (cargo build --release --bin curate-export),
# network access, python3.
set -euo pipefail
HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN="$HERE/target/release/curate-export"
[ -x "$BIN" ] || { echo "build first: (cd cli && cargo build --release --bin curate-export)" >&2; exit 2; }
WORK=${1:-$(mktemp -d)}
mkdir -p "$WORK"
USDC=0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
COMMON=(--anchor-rpc https://rpc.gnosischain.com --anchor-rpc https://gnosis-rpc.publicnode.com
        --log-rpc https://rpc.gnosischain.com --log-rpc https://gnosis.gateway.tenderly.co
        --provider-rpc https://gnosis-rpc.publicnode.com
        --gateway https://trustless-gateway.net --gateway https://cdn.kleros.link --gateway https://ipfs.filebase.io --gateway https://dweb.link --gateway https://ipfs.io)

for reg in address-tags tokens cdn atq; do
  echo "== export $reg =="
  start=$(date +%s)
  "$BIN" export --registry "$reg" "${COMMON[@]}" --out "$WORK/$reg.json" --provenance "$WORK/$reg-provenance.json" --csv "$WORK/$reg.csv" > "$WORK/$reg-summary.json"
  end=$(date +%s)
  python3 - "$WORK/$reg-summary.json" "$((end-start))" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))
print(f"  anchor {s['anchorBlock']} | logs {s['logs']} | unique {s['uniqueItems']} | by status {s['byStatus']} | included {s['included']} | fetch failures {s['fetchFailures']} | rpc calls {s['rpcCalls']} | {sys.argv[2]} s")
PY
done

echo "== tokenlist (Tokens snapshot, compared with t2crtokens.eth) =="
"$BIN" tokenlist --items "$WORK/tokens.json" --compare https://t2crtokens.eth.limo/ --out "$WORK/tokenlist.json" --diff-out "$WORK/diff.json" > "$WORK/tokenlist-summary.json"
python3 - "$WORK" "$USDC" <<'PY'
import json, sys
work, usdc = sys.argv[1], sys.argv[2].lower()
s = json.load(open(f"{work}/tokenlist-summary.json")); ours = json.load(open(f"{work}/tokenlist.json")); diff = json.load(open(f"{work}/diff.json"))
mine = {(t["chainId"], t["address"].lower()) for t in ours["tokens"]}
ours_only = {(t["chainId"], t["address"].lower()) for t in diff["oursOnly"]}
print(f"  tokens {s['tokens']} | skipped {s['skipped']} {s['skipReasons']} | version {s['version']} | sha256 {s['tokensSha256'][:16]}…")
print(f"  USDC mainnet in ours: {(1, usdc) in mine} | in theirs: {(1, usdc) in mine and (1, usdc) not in ours_only}")
print(f"  diff vs t2crtokens.eth ({s['compare']['referenceTokens']} tokens): ours-only {len(diff['oursOnly'])}, theirs-only {len(diff['theirsOnly'])}, metadata changes {len(diff['changed'])}")
PY

echo "== lookup: USDC's address in Address Tags =="
"$BIN" lookup --items "$WORK/address-tags.json" --address "$USDC" | head -12

echo "== determinism: a second Tokens export =="
"$BIN" export --registry tokens "${COMMON[@]}" --out "$WORK/tokens-2.json" --provenance "$WORK/tokens-2-provenance.json" > "$WORK/tokens-2-summary.json"
python3 - "$WORK" <<'PY'
import json, sys, hashlib
work = sys.argv[1]
a, b = json.load(open(f"{work}/tokens.json")), json.load(open(f"{work}/tokens-2.json"))
same_anchor = a["anchor"]["number"] == b["anchor"]["number"]
ba, bb = open(f"{work}/tokens.json","rb").read(), open(f"{work}/tokens-2.json","rb").read()
print(f"  anchors {a['anchor']['number']} / {b['anchor']['number']}; byte-identical: {ba == bb}; items identical with the anchor header removed: {a['items'] == b['items']}")
PY
echo "SMOKE-CURATE-EXPORT OK (work dir: $WORK)"
