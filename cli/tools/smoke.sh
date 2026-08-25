#!/bin/zsh
# Reproducible end-to-end smoke for the `intend` CLI against a local Gnosis
# fork, with a TWO-NODE header quorum: node A (8547) is an anvil fork of Gnosis
# at a pinned block; node B (8548) is an anvil fork OF NODE A, so both serve
# identical history from genuinely distinct normalized origins. B is re-forked
# after every on-chain mutation so the quorum's min-finalized height advances.
#
# What this proves: the full verified lifecycle (update → install → audit →
# revocation → re-registration → sticky re-enable) plus quorum plumbing with
# distinct origins. What it does NOT prove: operator independence (both nodes
# are local; `anchor_operators` labels them honestly as two local instances),
# or header-authenticated state roots (anvil fork headers carry ZERO state
# roots — the profile sets the loudly-labeled test_headerless_state_root
# workaround, refused on real chains).
#
# Requirements: anvil (foundry), cast, python3, the Gate 2 spike crate at
# ../spikes/snapshot-bench (seeder), and a built `intend` binary.
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/.." && pwd)"
work="${SMOKE_WORK:-$(mktemp -d)}"
mkdir -p "$work"
A=http://127.0.0.1:8547
B=http://127.0.0.1:8548
KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
FORK_BLOCK=47881774
UPSTREAM=https://rpc.gnosischain.com
INTEND="$here/target/release/intend"

pids=()
cleanup() { for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null || true; done }
trap cleanup EXIT

start_a() {
  anvil --fork-url "$UPSTREAM" --fork-block-number $FORK_BLOCK --port 8547 \
        --slots-in-an-epoch 1 > "$work/anvil-a.log" 2>&1 &
  pids+=($!)
  sleep 6
}

refork_b() {
  # (Re)fork B from A's CURRENT latest so both serve identical history.
  if [ -n "${B_PID:-}" ]; then kill "$B_PID" 2>/dev/null || true; sleep 1; fi
  anvil --fork-url "$A" --port 8548 --slots-in-an-epoch 1 \
        > "$work/anvil-b.log" 2>&1 &
  B_PID=$!
  pids+=($B_PID)
  sleep 5
}

settle() { # mine the 2-block finality margin on A, then re-fork B
  cast rpc anvil_mine 0x2 --rpc-url $A > /dev/null
  refork_b
}

echo "== node A (fork of Gnosis @ $FORK_BLOCK) =="
start_a
# Warp A's clock to now (fork timestamps are stale; freshness fails closed).
now=$(date +%s)
ts=$(cast block latest --rpc-url $A --json | python3 -c "import json,sys; print(int(json.load(sys.stdin)['timestamp'],16))")
cast rpc evm_increaseTime $((now - ts)) --rpc-url $A > /dev/null
cast rpc anvil_mine 0x2 --rpc-url $A > /dev/null

echo "== seed registry (Gate 2 spike) =="
( cd "$repo/spikes/snapshot-bench" && \
  cargo run --release --locked -q -- seed --rpc-url $A --items 25 \
    --out "$work/seed-manifest.json" > /dev/null )
REG=$(python3 -c "import json;print(json.load(open('$work/seed-manifest.json'))['registry'])")
CODEHASH=$(python3 -c "import json;print(json.load(open('$work/seed-manifest.json'))['registryCodeHash'])")

echo "== register the kubo-interop item =="
CID=$(cat "$here/fixtures/kubo/root-cid.txt")
RLP=$(python3 - "$CID" <<'EOF'
import sys
def enc(sv):
    b = sv.encode()
    if len(b) == 1 and b[0] < 0x80: return b
    if len(b) <= 55: return bytes([0x80+len(b)]) + b
    ln = len(b).to_bytes((len(b).bit_length()+7)//8, 'big')
    return bytes([0xb7+len(ln)]) + ln + b
cols = ["kubo-interop-skill", "End-to-end install smoke item.", sys.argv[1], "generic", "", ""]
payload = b''.join(enc(c) for c in cols)
if len(payload) <= 55: out = bytes([0xc0+len(payload)]) + payload
else:
    ln = len(payload).to_bytes((len(payload).bit_length()+7)//8, 'big')
    out = bytes([0xf7+len(ln)]) + ln + payload
print('0x'+out.hex())
EOF
)
ID=$(cast keccak "$RLP")
EXTRA=0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003
COST=$(cast call 0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002 "arbitrationCost(bytes)(uint256)" $EXTRA --rpc-url $A)
COST=${COST%% *}
cast send $REG "addItem(bytes)" "$RLP" --value $COST --private-key $KEY --rpc-url $A --json > /dev/null
cast rpc evm_increaseTime 10 --rpc-url $A > /dev/null && cast rpc evm_mine --rpc-url $A > /dev/null
cast send $REG "executeRequest(bytes32)" "$ID" --private-key $KEY --rpc-url $A --json > /dev/null
settle

echo "== profile (two distinct origins; workaround labeled) =="
GENESIS=$(cast block 0 --rpc-url $A --json | python3 -c "import json,sys; print(json.load(sys.stdin)['hash'])")
cat > "$work/profile.toml" <<EOF
chain_id = 100
genesis_hash = "$GENESIS"
registry = "$REG"
registry_code_hash = "$CODEHASH"
anchor_rpcs = ["$A", "$B"]
anchor_operators = ["local-anvil-a", "local-anvil-b"]
registration_meta_evidence = "/ipfs/$CID/registration.json"
clearing_meta_evidence = "/ipfs/$CID/clearing.json"
provider_rpc = "$A"
gateways = []
test_headerless_state_root = true
EOF
P=(--profile "$work/profile.toml" --state-dir "$work/state")

echo "== update =="
"$INTEND" "${P[@]}" update
echo "== install =="
( cd "$work" && "$INTEND" "${P[@]}" install kubo-interop-skill \
    --dir ./skill --car "$here/fixtures/kubo/tree.car" )
echo "== audit (expect current, exit 0) =="
( cd "$work" && "$INTEND" "${P[@]}" audit )

echo "== remove on-chain, audit (expect revoked, exit 1) =="
cast send $REG "removeItem(bytes32,string)" "$ID" "" --value $COST --private-key $KEY --rpc-url $A --json > /dev/null
cast rpc evm_increaseTime 10 --rpc-url $A > /dev/null && cast rpc evm_mine --rpc-url $A > /dev/null
cast send $REG "executeRequest(bytes32)" "$ID" --private-key $KEY --rpc-url $A --json > /dev/null
settle
set +e
( cd "$work" && "$INTEND" "${P[@]}" audit ); rc=$?
set -e
[ $rc -eq 1 ] || { echo "FATAL: expected audit exit 1, got $rc"; exit 1; }

echo "== re-register, audit (expect reenable-required — sticky), enable, audit (current) =="
cast send $REG "addItem(bytes)" "$RLP" --value $COST --private-key $KEY --rpc-url $A --json > /dev/null
cast rpc evm_increaseTime 10 --rpc-url $A > /dev/null && cast rpc evm_mine --rpc-url $A > /dev/null
cast send $REG "executeRequest(bytes32)" "$ID" --private-key $KEY --rpc-url $A --json > /dev/null
settle
set +e
( cd "$work" && "$INTEND" "${P[@]}" audit ); rc=$?
set -e
[ $rc -eq 1 ] || { echo "FATAL: sticky suspension must keep audit failing, got $rc"; exit 1; }
( cd "$work" && "$INTEND" "${P[@]}" enable ./skill )
( cd "$work" && "$INTEND" "${P[@]}" audit )
echo "SMOKE OK (work dir: $work)"
