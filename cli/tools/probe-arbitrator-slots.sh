#!/bin/zsh
# ASSERTING reproduction of docs/spikes/arbitrator-slot-evidence.md on an anvil
# Gnosis fork: deploys a registry via the REAL GTCRFactory (the Gate 2 spike's
# `seed`), then PROVES (or fails loudly) each pinned claim about the two storage
# slots the `intend` verifier pins (spec §5, §6 step 3c):
#   A. the deployed runtime codehash equals the pinned constant;
#   B. slot 0 holds arbitrator() (upper 12 bytes zero);
#   C. slot 1 holds the LONG-form word len*2+1 for the deployed 64-byte extra
#      data, and keccak256(1)+0, +1 hold its two words, equal to
#      arbitratorExtraData();
#   D. a governor changeArbitrator to a dummy with 3-byte extra data moves slot 0
#      to the dummy and slot 1 to the SHORT form (data inline, low byte len*2),
#      while every other SAMPLED slot (2..16) is byte-identical;
#   E. a governor changeArbitrator back to the arbitrator with 64-byte data
#      restores the long form and the data words, again with slots 2..16 unchanged.
# Any violated assertion exits nonzero. A transcript of asserted values is written
# to $work/transcript.txt (sha256 printed; copied to $PROBE_OUT if set).
# Requires anvil, cast, python3, and the Gate 2 spike crate.
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/.." && pwd)"
work="$(mktemp -d)"
RPC=http://127.0.0.1:8549
KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
PINNED_CODEHASH=0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d
ARBITRATOR=0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002
UPSTREAM=${UPSTREAM:-https://rpc.gnosischain.com}
T="$work/transcript.txt"
log() { echo "$*" | tee -a "$T"; }
fail() { log "FAIL: $*"; exit 1; }

anvil --fork-url "$UPSTREAM" --port 8549 --silent > "$work/anvil.log" 2>&1 &
APID=$!
trap 'kill $APID 2>/dev/null || true' EXIT
for i in $(seq 1 60); do cast block-number --rpc-url $RPC >/dev/null 2>&1 && break; sleep 1; done

log "== A. deploy via the real factory (spike seed) and pin the codehash =="
( cd "$repo/spikes/snapshot-bench" && cargo run --release --locked -q -- seed --rpc-url $RPC --items 1 --out "$work/seed.json" > /dev/null )
REG=$(python3 -c "import json;print(json.load(open('$work/seed.json'))['registry'])")
CH=$(cast keccak "$(cast code $REG --rpc-url $RPC)")
log "registry=$REG codehash=$CH"
[ "$CH" = "$PINNED_CODEHASH" ] || fail "codehash $CH != pinned $PINNED_CODEHASH"

sample() { for s in $(seq 2 16); do echo "$s:$(cast storage $REG $s --rpc-url $RPC)"; done; }
word() { cast storage $REG "$1" --rpc-url $RPC; }
K1=$(cast keccak 0x0000000000000000000000000000000000000000000000000000000000000001)
K1B=$(python3 -c "print(hex(int('$K1',16)+1))")

log "== B/C. slot 0 = arbitrator(); slot 1 + keccak(1)+i = arbitratorExtraData() (long form) =="
ARB=$(cast call $REG "arbitrator()(address)" --rpc-url $RPC)
ED=$(cast call $REG "arbitratorExtraData()(bytes)" --rpc-url $RPC)
S0=$(word 0); S1=$(word 1); D0=$(word $K1); D1=$(word $K1B)
log "arbitrator()=$ARB slot0=$S0"; log "extraData=$ED slot1=$S1 data0=$D0 data1=$D1"
python3 - "$ARB" "$ED" "$S0" "$S1" "$D0" "$D1" <<'PY' || fail "B/C"
import sys
arb, ed, s0, s1, d0, d1 = [x.lower() for x in sys.argv[1:]]
assert s0 == "0x" + "0"*24 + arb[2:], "slot 0 is not the arbitrator"
data = bytes.fromhex(ed[2:]); assert len(data) == 64, "expected 64-byte extra data"
assert int(s1, 16) == len(data)*2 + 1, "slot 1 is not the long-form length word"
assert bytes.fromhex(d0[2:]) == data[:32] and bytes.fromhex(d1[2:]) == data[32:], "data words differ from the getter"
print("B/C ok")
PY
BASE=$(sample)

log "== D. changeArbitrator(dummy, 0xaabbcc): short form, slot 0 moves, slots 2..16 unchanged =="
cast send $REG "changeArbitrator(address,bytes)" 0x000000000000000000000000000000000000dEaD 0xaabbcc --private-key $KEY --rpc-url $RPC --json > "$work/tx1.json"
S0=$(word 0); S1=$(word 1); log "slot0=$S0 slot1=$S1"
[ "$S0" = "0x000000000000000000000000000000000000000000000000000000000000dead" ] || fail "D: slot 0"
[ "$S1" = "0xaabbcc0000000000000000000000000000000000000000000000000000000006" ] || fail "D: slot 1 short form"
[ "$(sample)" = "$BASE" ] || fail "D: another sampled slot moved"

log "== E. changeArbitrator back with 64-byte data: long form restored =="
EXTRA=$(printf "0x%064x%064x" 19 3)
cast send $REG "changeArbitrator(address,bytes)" $ARBITRATOR $EXTRA --private-key $KEY --rpc-url $RPC --json > "$work/tx2.json"
S0=$(word 0); S1=$(word 1); D0=$(word $K1); D1=$(word $K1B); log "slot0=$S0 slot1=$S1 data0=$D0 data1=$D1"
[ "$S0" = "0x0000000000000000000000009c1da9a04925bdfdedf0f6421bc7eea8305f9002" ] || fail "E: slot 0"
[ "$S1" = "0x0000000000000000000000000000000000000000000000000000000000000081" ] || fail "E: slot 1"
[ "$D0" = "0x0000000000000000000000000000000000000000000000000000000000000013" ] || fail "E: data0"
[ "$D1" = "0x0000000000000000000000000000000000000000000000000000000000000003" ] || fail "E: data1"
[ "$(sample)" = "$BASE" ] || fail "E: another sampled slot moved"

log "ALL ASSERTIONS PASSED"
echo "transcript sha256: $(shasum -a 256 "$T" | cut -d' ' -f1)"
[ -n "${PROBE_OUT:-}" ] && cp "$T" "$PROBE_OUT" && echo "copied to $PROBE_OUT" || true
