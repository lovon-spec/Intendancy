#!/bin/zsh
# Reproducible end-to-end smoke for the `intend` CLI against a local Gnosis
# fork, with a TWO-NODE header quorum: node A (8547) is an anvil fork of Gnosis
# at a pinned block; node B (8548) is an anvil fork OF NODE A, so both serve
# identical history from genuinely distinct normalized origins. B is re-forked
# after every on-chain mutation so the quorum's min-finalized height advances.
#
# What this proves: the full verified lifecycle (update → install → audit →
# revocation → re-registration → sticky re-enable → policy-version transition
# with `migrate`, including a quarantined tree migrating in a blocked state)
# plus quorum plumbing with distinct origins. What it does NOT prove: operator independence (both nodes
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
arbitrator = "$(cast call $REG "arbitrator()(address)" --rpc-url $A)"
arbitrator_extra_data = "$(cast call $REG "arbitratorExtraData()(bytes)" --rpc-url $A)"
governor = "$(cast call $REG "governor()(address)" --rpc-url $A)"
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
echo "== a second install, then quarantined the way the bootstrap skill does (its own lockfile) =="
( cd "$work" && "$INTEND" "${P[@]}" install kubo-interop-skill \
    --dir ./skill2 --car "$here/fixtures/kubo/tree.car" --lockfile ./quarantine-lock.json )
# One audit while intact, so the entry carries an audit record the migration must keep.
( cd "$work" && "$INTEND" "${P[@]}" audit --lockfile ./quarantine-lock.json > /dev/null )
mv "$work/skill2" "$work/quarantine-skill2"
echo "== policy version and governor pins (owner decision 2026-09-06) =="
# The governor (the deployer in mock mode) announces a new policy: the counter moves to 1.
cast send $REG "changeMetaEvidence(string,string)" "/ipfs/$CID/registration-v1.json" "/ipfs/$CID/clearing-v1.json" --private-key $KEY --rpc-url $A --json > /dev/null
settle
set +e
( "$INTEND" "${P[@]}" update > "$work/update-v1-old.log" 2>&1 ); rc=$?
set -e
[ $rc -eq 1 ] || { echo "FATAL: the old profile must fail closed on policy version 1, got $rc"; cat "$work/update-v1-old.log"; exit 1; }
grep -q "policy version 1 is not one the profile accepts" "$work/update-v1-old.log" || { echo "FATAL: unexpected failure text"; cat "$work/update-v1-old.log"; exit 1; }
echo "old profile fails closed on policy version 1: ok"
# A new signed profile that names version 1 verifies (a new context: fresh state dir).
cp "$work/profile.toml" "$work/profile-v1.toml"
cat >> "$work/profile-v1.toml" <<EOF

[[policy_updates]]
updates = 1
registration_meta_evidence = "/ipfs/$CID/registration-v1.json"
clearing_meta_evidence = "/ipfs/$CID/clearing-v1.json"
EOF
P1=(--profile "$work/profile-v1.toml" --state-dir "$work/state-v1")
"$INTEND" "${P1[@]}" update
echo "profile naming version 1 verifies: ok"

echo "== migrate: the old profile's entries are refused by the new one until carried across =="
set +e
( cd "$work" && "$INTEND" "${P1[@]}" audit > "$work/audit-v1-before.log" 2>&1 ); rc=$?
set -e
[ $rc -ne 0 ] || { echo "FATAL: the new profile must refuse old-context entries before migration"; cat "$work/audit-v1-before.log"; exit 1; }
grep -q "intend migrate --from" "$work/audit-v1-before.log" || { echo "FATAL: the refusal must name the migration"; cat "$work/audit-v1-before.log"; exit 1; }
echo "new profile refuses old-context entries and names the migration: ok"
# The intact install migrates current; the new profile audits it; the old one no longer does.
( cd "$work" && "$INTEND" "${P1[@]}" migrate --from "$work/profile.toml" > "$work/migrate.log" ); cat "$work/migrate.log"
python3 - "$work/migrate.log" <<'EOF'
import json, sys
r = json.loads(open(sys.argv[1]).read().strip().splitlines()[-1])
assert r["ok"] and r["migrated"] == 1 and r["skipped"] == 0 and r["states"]["current"] == 1, r
assert r["states"]["entries"][0]["localIntegrity"] == "intact", r
EOF
( cd "$work" && "$INTEND" "${P1[@]}" audit )
set +e
( cd "$work" && "$INTEND" "${P[@]}" audit > "$work/audit-old-after.log" 2>&1 ); rc=$?
set -e
[ $rc -ne 0 ] || { echo "FATAL: the old profile must refuse migrated entries"; exit 1; }
grep -q "intend migrate --from" "$work/audit-old-after.log" || { echo "FATAL: unexpected refusal text"; cat "$work/audit-old-after.log"; exit 1; }
echo "migrated lockfile: new profile audits it (current), old profile refuses it: ok"
# A second run is idempotent: everything is already at the new context.
( cd "$work" && "$INTEND" "${P1[@]}" migrate --from "$work/profile.toml" > "$work/migrate-again.log" )
python3 - "$work/migrate-again.log" <<'EOF'
import json, sys
r = json.loads(open(sys.argv[1]).read().strip().splitlines()[-1])
assert r["ok"] and r["migrated"] == 0 and r["skipped"] == 1, r
EOF
echo "repeat migration skips entries already at the new context: ok"
# The quarantined tree migrates in a blocked state: context and history move,
# the bytes stay unauthorized, and audit keeps reporting it.
( cd "$work" && "$INTEND" "${P1[@]}" migrate --from "$work/profile.toml" --lockfile ./quarantine-lock.json > "$work/migrate-q.log" ); cat "$work/migrate-q.log"
python3 - "$work/migrate-q.log" <<'EOF'
import json, sys
r = json.loads(open(sys.argv[1]).read().strip().splitlines()[-1])
assert r["ok"] and r["migrated"] == 1 and r["states"]["modified"] == 1, r
e = r["states"]["entries"][0]
assert e["state"] == "modified" and "missing" in e["localIntegrity"], e
EOF
set +e
( cd "$work" && "$INTEND" "${P1[@]}" audit --lockfile ./quarantine-lock.json > "$work/audit-q.log" 2>&1 ); rc=$?
set -e
[ $rc -eq 1 ] || { echo "FATAL: a quarantined tree must keep audit failing, got $rc"; cat "$work/audit-q.log"; exit 1; }
grep -q '"state":"modified"' "$work/audit-q.log" || grep -q '"state": "modified"' "$work/audit-q.log" || { echo "FATAL: audit must report the quarantined tree as modified"; cat "$work/audit-q.log"; exit 1; }
python3 - "$work/quarantine-lock.json" <<'EOF'
import json, sys
lock = json.load(open(sys.argv[1]))
e = lock["entries"][0]
assert len(e["migrations"]) == 1 and "missing" in e["migrations"][0]["localIntegrity"], e
assert e["migrations"][0]["previousAudit"]["state"] == "current", e["migrations"][0]
assert len(e["files"]) > 0, "manifest preserved"
EOF
echo "quarantined tree migrates blocked (modified, missing), manifest and audit history kept: ok"
# A governor switch fails closed under that profile until a release pins the new governor.
ACCT1=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
KEY1=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
cast send $REG "changeGovernor(address)" $ACCT1 --private-key $KEY --rpc-url $A --json > /dev/null
settle
set +e
( "$INTEND" "${P1[@]}" update > "$work/update-gov.log" 2>&1 ); rc=$?
set -e
[ $rc -eq 1 ] || { echo "FATAL: a switched governor must fail closed, got $rc"; cat "$work/update-gov.log"; exit 1; }
grep -q "governor pin violated" "$work/update-gov.log" || { echo "FATAL: unexpected failure text"; cat "$work/update-gov.log"; exit 1; }
cast send $REG "changeGovernor(address)" "$(cast wallet address --private-key $KEY)" --private-key $KEY1 --rpc-url $A --json > /dev/null
settle
"$INTEND" "${P1[@]}" update
echo "governor switch fails closed, verifies again once restored: ok"
echo "SMOKE OK (work dir: $work)"
