#!/usr/bin/env bash
# Live smoke for `curate-export`: snapshot all four Kleros Scout registries on
# Gnosis against public RPCs and gateways, render the Tokens list as a token
# list with a diff against the t2crtokens.eth list (Kleros's archived
# exporter's output, still regenerated), and run an offline lookup on Address
# Tags. Prints wall-clock time and the measured RPC, IPFS and per-stage
# counters per registry.
#
# Assertions (a violation fails the run):
#   - every export reports ok (the tool fails closed when the two log sources
#     disagree or a proof does not verify);
#   - same-anchor determinism: the Address Tags Query export rerun at the
#     first run's anchor (--anchor-block) is byte-identical to the first;
#   - the token list carries the default name and was written (the tool
#     validates it against the token-lists schema before writing).
# Informational (different finalized anchors can legitimately differ):
#   - the diff against t2crtokens.eth and the USDC lookup;
#   - a second Tokens export at the then-current anchor, compared with the
#     first.
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

fail() { echo "SMOKE-CURATE-EXPORT FAIL: $*" >&2; exit 1; }

summarize() {
  python3 - "$1" "$2" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))
assert s["ok"] is True, s
r, i, st = s["rpc"], s["ipfs"], s["stageSeconds"]
print(f"  anchor {s['anchorBlock']} ({s['anchorSource']}) | logs {s['logs']} | unique {s['uniqueItems']} | by status {s['byStatus']} | included {s['included']} | fetch failures {s['fetchFailures']} | {sys.argv[2]} s")
print(f"  rpc: {r['requests']} requests, {r['attempts']} attempts, {r['retries']} retries, {r['responseBytes']} bytes")
print(f"  ipfs: {i['requests']} block requests, {i['responseBytes']} bytes, {i['badBytes']} bad-byte responses, {i['cacheHits']} cache hits, {i['coalesced']} coalesced, {i['itemRetries']} item retries")
print(f"  stages (s): identity {st['identity']:.1f}, anchor {st['anchor']:.1f}, enumeration {st['enumeration']:.1f}, proofs {st['proofs']:.1f}, content {st['content']:.1f}")
PY
}

for reg in address-tags tokens cdn atq; do
  echo "== export $reg =="
  start=$(date +%s)
  "$BIN" export --registry "$reg" "${COMMON[@]}" --out "$WORK/$reg.json" --provenance "$WORK/$reg-provenance.json" --csv "$WORK/$reg.csv" > "$WORK/$reg-summary.json" || fail "export $reg failed"
  end=$(date +%s)
  summarize "$WORK/$reg-summary.json" "$((end-start))" || fail "export $reg did not report ok"
done

echo "== same-anchor determinism: Address Tags Query again at its anchor (assertion) =="
ANCHOR=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["anchorBlock"])' "$WORK/atq-summary.json")
"$BIN" export --registry atq "${COMMON[@]}" --anchor-block "$ANCHOR" --out "$WORK/atq-again.json" --provenance "$WORK/atq-again-provenance.json" > "$WORK/atq-again-summary.json" || fail "atq export at anchor $ANCHOR failed"
cmp -s "$WORK/atq.json" "$WORK/atq-again.json" || fail "atq export rerun at anchor $ANCHOR is not byte-identical to the first (items.json differs)"
echo "  byte-identical items.json at anchor $ANCHOR ($(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["anchorSource"])' "$WORK/atq-again-summary.json"))"

echo "== tokenlist (Tokens snapshot, compared with t2crtokens.eth) =="
"$BIN" tokenlist --items "$WORK/tokens.json" --compare https://t2crtokens.eth.limo/ --out "$WORK/tokenlist.json" --diff-out "$WORK/diff.json" --skipped-out "$WORK/tokenlist-skipped.json" > "$WORK/tokenlist-summary.json" || fail "tokenlist failed"
python3 - "$WORK" "$USDC" <<'PY' || fail "token list assertions"
import json, sys
work, usdc = sys.argv[1], sys.argv[2].lower()
s = json.load(open(f"{work}/tokenlist-summary.json")); ours = json.load(open(f"{work}/tokenlist.json")); diff = json.load(open(f"{work}/diff.json"))
assert s["ok"] is True, s
assert ours["name"] == "Kleros Tokens Verified" == s["name"], ours["name"]
assert 1 <= len(ours["tokens"]) <= 10000 and ours["tokens"] and ours["version"]["major"] >= 1
mine = {(t["chainId"], t["address"].lower()) for t in ours["tokens"]}
ours_only = {(t["chainId"], t["address"].lower()) for t in diff["oursOnly"]}
theirs_only = {(t["chainId"], t["address"].lower()) for t in diff["theirsOnly"]}
print(f"  name {ours['name']!r} | tokens {s['tokens']} | skipped {s['skipped']} {s['skipReasons']} | version {s['version']} | sha256 {s['tokensSha256'][:16]}…")
print(f"  USDC mainnet in ours: {(1, usdc) in mine} | in theirs: {((1, usdc) in mine and (1, usdc) not in ours_only) or (1, usdc) in theirs_only}")
print(f"  diff vs t2crtokens.eth ({s['compare']['referenceTokens']} tokens, informational): ours-only {len(diff['oursOnly'])}, theirs-only {len(diff['theirsOnly'])}, metadata changes {len(diff['changed'])}")
PY

echo "== lookup: USDC's address in Address Tags (informational) =="
"$BIN" lookup --items "$WORK/address-tags.json" --address "$USDC" | head -12

echo "== a second Tokens export at the current anchor (informational: anchors differ) =="
"$BIN" export --registry tokens "${COMMON[@]}" --out "$WORK/tokens-2.json" --provenance "$WORK/tokens-2-provenance.json" > "$WORK/tokens-2-summary.json" || fail "second tokens export failed"
python3 - "$WORK" <<'PY'
import json, sys
work = sys.argv[1]
a, b = json.load(open(f"{work}/tokens.json")), json.load(open(f"{work}/tokens-2.json"))
ba, bb = open(f"{work}/tokens.json","rb").read(), open(f"{work}/tokens-2.json","rb").read()
print(f"  anchors {a['anchor']['number']} / {b['anchor']['number']}; byte-identical: {ba == bb}; items identical with the anchor header removed: {a['items'] == b['items']} ({len(a['items'])} / {len(b['items'])} items)")
PY
echo "SMOKE-CURATE-EXPORT OK (work dir: $WORK)"
