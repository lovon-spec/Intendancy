#!/bin/zsh
# Rehearse the Agent Skills Registry's switch to an Intendment wrapper on an anvil fork of Gnosis
# and run the intend CLI across it: the real registry, the real timelock (impersonated) deploying
# a wrapper through IntendmentFactory, registering its epoch and calling
# changeArbitrator(wrapper, envelope), as the governance batch will.
#
# Fork mode proves only state anvil holds locally (a forked account's remote slots read as absent
# in eth_getProof), so the script first copies the registry slots the CLI proves into local
# storage, with the values the fork reads for them.
#
# What it shows: before the switch the shipped profile and a profile that also pins the wrapper
# both verify through the court; after it the shipped profile fails closed and the wrapper profile
# verifies (envelope proven in slot 1, the wrapper's code hash proven at the same anchor), carries
# an install across with `migrate --from`, and audits it current; a submission paying the deposit
# computed from the registry's live arbitrator and extra data is accepted, executes, and verifies;
# a second wrapper instance fails closed. What it does not show: quorum independence (one anvil
# node under two origins) or header-authenticated state roots (fork headers carry none, so the
# profiles set the labelled test workaround).
#
# Requirements: anvil, cast, forge, python3, a built intend (`cargo build`), and the intendment
# repository at $INTENDMENT, built into the work dir (the repository itself is not written).
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
INTENDMENT=${INTENDMENT:?set INTENDMENT to the intendment repository}
work="${WORK:-$(mktemp -d)}"
mkdir -p "$work"
A=http://127.0.0.1:8547
ALIAS=http://0.0.0.0:8547
UPSTREAM=${UPSTREAM:-https://rpc.gnosischain.com}
FORK_BLOCK=${FORK_BLOCK:-48859000}
INTEND=${INTEND:-$here/target/debug/intend}
REG=0x67DBE6A9597635074546e08B92eE617bF02168f9
COURT=0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002
TIMELOCK=0xc8Ba4c0AD3554EDB0a9A4C8D73Bf87410A313ADa
EXTRA=0x00000000000000000000000000000000000000000000000000000000000000130000000000000000000000000000000000000000000000000000000000000003
KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 # anvil's first dev account

pid=
cleanup() { [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; }
trap cleanup EXIT
fail() { echo "FATAL: $*"; exit 1; }
num() { echo "${1%% *}"; }
settle() { cast rpc anvil_mine 0x2 --rpc-url $A > /dev/null; } # past the fork's finality margin
send() { cast send "$@" --private-key $KEY --rpc-url $A --json > /dev/null; }
as_timelock() { cast send --from $TIMELOCK --unlocked "$@" --rpc-url $A --json; }
P_OLD=(--profile "$work/old.toml" --state-dir "$work/state-old")
P_NEW=(--profile "$work/new.toml" --state-dir "$work/state-new")
lower() { echo "$1" | tr 'A-F' 'a-f'; }
json_field() { python3 -I -c 'import json,sys; print(json.loads(open(sys.argv[1]).read().strip().splitlines()[-1])[sys.argv[2]])' "$@"; }
expect_fail() { # <log> <needle> <intend args...>: must exit nonzero naming <needle>
  local log=$1 needle=$2; shift 2
  set +e; "$INTEND" "$@" > "$log" 2>&1; local rc=$?; set -e
  [ $rc -ne 0 ] || fail "expected failure, got success: $*"
  grep -q "$needle" "$log" || { cat "$log"; fail "expected \"$needle\""; }
}

# The deposit a request pays: the base deposit plus what the registry's LIVE arbitrator quotes for
# its LIVE extra data (Classic GTCR charges request.arbitrator.arbitrationCost(extraData)).
deposit() {
  local arb=$(cast call $REG "arbitrator()(address)" --rpc-url $A)
  local extra=$(cast call $REG "arbitratorExtraData()(bytes)" --rpc-url $A)
  local base=$(num "$(cast call $REG "$1()(uint256)" --rpc-url $A)")
  local cost=$(num "$(cast call $arb "arbitrationCost(bytes)(uint256)" $extra --rpc-url $A)")
  python3 -I -c 'import sys; print(int(sys.argv[1]) + int(sys.argv[2]))' $base $cost
}
slot() { python3 -I -c 'import sys; print("0x%064x" % (int(sys.argv[1], 16) + int(sys.argv[2])))' $1 ${2:-0}; }
materialize() { # the proven registry slots, made local with the values the fork reads
  local -a slots=($(slot 0x0) $(slot 0x1) $(slot 0x3) $(slot 0x9) $(slot 0xd))
  local data=$(cast keccak $(slot 0x1)) list=$(cast keccak $(slot 0xd))
  local n=$(python3 -I -c 'import sys; print(int(sys.argv[1], 16))' $(cast storage $REG 13 --rpc-url $A))
  for i in {0..5}; do slots+=($(slot $data $i)); done
  for ((i = 0; i < n; i++)); do
    local id=$(cast storage $REG $(slot $list $i) --rpc-url $A)
    slots+=($(slot $list $i) $(slot $(cast index bytes32 $id 14) 1))
  done
  for s in $slots; do
    cast rpc anvil_setStorageAt $REG $s $(cast storage $REG $s --rpc-url $A) --rpc-url $A > /dev/null
  done
}
descriptor() { # RLP of the six policy columns: name, description, Tree CID, runtimes, origin, reserved
  python3 -I - "$@" <<'EOF'
import sys
def enc(sv):
    b = sv.encode()
    if len(b) == 1 and b[0] < 0x80: return b
    if len(b) <= 55: return bytes([0x80 + len(b)]) + b
    ln = len(b).to_bytes((len(b).bit_length() + 7) // 8, 'big')
    return bytes([0xb7 + len(ln)]) + ln + b
payload = b''.join(enc(c) for c in [sys.argv[1], sys.argv[2], sys.argv[3], "generic", "", ""])
if len(payload) <= 55: out = bytes([0xc0 + len(payload)]) + payload
else:
    ln = len(payload).to_bytes((len(payload).bit_length() + 7) // 8, 'big')
    out = bytes([0xf7 + len(ln)]) + ln + payload
print('0x' + out.hex())
EOF
}
register() { # <descriptor>: request, wait out the (shortened) challenge period, execute
  send $REG "addItem(bytes)" "$1" --value "$(deposit submissionBaseDeposit)"
  cast rpc evm_increaseTime 11 --rpc-url $A > /dev/null && cast rpc evm_mine --rpc-url $A > /dev/null
  send $REG "executeRequest(bytes32)" "$(cast keccak "$1")"
  settle
}
deploy_wrapper() { # the timelock asks the factory for an instance bound to the registry and court
  as_timelock $FACTORY "deploy(address,address,uint8,(uint256,uint256,uint256,uint256,uint256))" \
    $REG $COURT 1 "(172800,86400,86400,604800,86400)" \
    | python3 -I -c 'import json,sys; t=sys.argv[1]; r=json.load(sys.stdin); print("0x" + [l for l in r["logs"] if l["topics"][0] == t][0]["topics"][2][-40:])' \
      "$(cast keccak "WrapperDeployed(address,address,address)")"
  # (window 2 d, funding 1 d, resolver grace 1 d, reserve timelock 7 d, retirement margin 1 d)
}

echo "== build the wrapper contracts =="
forge build --root "$INTENDMENT" --out "$work/intendment-out" --cache-path "$work/intendment-cache" \
  > "$work/forge-build.log" 2>&1 || { cat "$work/forge-build.log"; fail "forge build"; }
FACTORY_CODE=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["bytecode"]["object"])' \
  "$work/intendment-out/IntendmentFactory.sol/IntendmentFactory.json")

echo "== anvil fork of Gnosis @ $FORK_BLOCK on $A =="
anvil --fork-url "$UPSTREAM" --fork-block-number $FORK_BLOCK --port 8547 --slots-in-an-epoch 1 \
      --retries 10 --fork-retry-backoff 2000 --compute-units-per-second 50 > "$work/anvil.log" 2>&1 &
pid=$!
for i in {1..60}; do cast chain-id --rpc-url $A > /dev/null 2>&1 && break; sleep 0.5; done
now=$(date +%s)
ts=$(cast block latest --rpc-url $A --json | python3 -I -c "import json,sys; print(int(json.load(sys.stdin)['timestamp'],16))")
cast rpc evm_increaseTime $((now - ts)) --rpc-url $A > /dev/null
cast rpc anvil_mine 0x4 --rpc-url $A > /dev/null # the finalized block is a local one (no header state root)
cast rpc anvil_impersonateAccount $TIMELOCK --rpc-url $A > /dev/null
cast rpc anvil_setBalance $TIMELOCK 0x3635C9ADC5DEA00000 --rpc-url $A > /dev/null
materialize

echo "== profiles: the shipped one, pointed at the fork =="
python3 -I - "$here/profiles/agent-skills-registry.toml" "$work/old.toml" $A $ALIAS <<'EOF'
import re, sys
src, dst, a, alias = sys.argv[1:5]
s = open(src).read()
def sub(key, value):
    global s
    s, n = re.subn(rf'(?m)^{key} = .*$', f'{key} = {value}', s)
    assert n == 1, key
sub('anchor_rpcs', f'["{a}", "{alias}"]')
sub('anchor_operators', '["local-anvil", "local-anvil-same-node"]')
sub('provider_rpc', f'"{a}"')
sub('gateways', '[]\ntest_headerless_state_root = true')
open(dst, 'w').write(s)
EOF

echo "== before the switch: register and install a skill under the shipped profile =="
# The fork shortens the challenge period so requests execute in seconds.
as_timelock $REG "changeTimeToChallenge(uint256)" 10 > /dev/null
CID=$(cat "$here/fixtures/kubo/root-cid.txt")
SKILL=$(descriptor kubo-interop-skill "End-to-end install smoke item." "$CID")
register "$SKILL"
"$INTEND" "${P_OLD[@]}" update
( cd "$work" && "$INTEND" "${P_OLD[@]}" install kubo-interop-skill --dir ./skill \
    --car "$here/fixtures/kubo/tree.car" --lockfile ./lock.json )
( cd "$work" && "$INTEND" "${P_OLD[@]}" audit --lockfile ./lock.json )

echo "== deploy the wrapper the way the governance batch will =="
FACTORY=$(cast send --private-key $KEY --rpc-url $A --create "$FACTORY_CODE" --json \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["contractAddress"])')
W=$(deploy_wrapper)
Q=$(num "$(cast call $COURT "arbitrationCost(bytes)(uint256)" $EXTRA --rpc-url $A)")
as_timelock $W "registerEpoch(bytes,uint256,uint256,uint256,string,string)" $EXTRA 0 0 $Q \
  "ipfs://fork-registration" "ipfs://fork-removal" > /dev/null
as_timelock $W "fundReserve()" --value $Q > /dev/null
ENVELOPE=$(cast abi-encode "f(uint8,uint32,bytes)" 1 1 $EXTRA)
settle
echo "wrapper $W (factory $FACTORY), epoch 1 quotes $Q wei"

echo "== pin it: a profile that accepts the court or this wrapper =="
"$here/tools/pin-wrapper.sh" $A $W 1 "$work/old.toml" > "$work/wrapper.toml"
cat "$work/wrapper.toml"
{ cat "$work/old.toml"; echo; cat "$work/wrapper.toml"; } > "$work/new.toml"
"$INTEND" "${P_NEW[@]}" update > "$work/update-new-before.log"; cat "$work/update-new-before.log"
[ "$(json_field "$work/update-new-before.log" arbitrator)" = "$(lower $COURT)" ] || fail "expected the court arm"

echo "== the switch: changeArbitrator(wrapper, envelope) through the timelock =="
as_timelock $REG "changeArbitrator(address,bytes)" $W $ENVELOPE > /dev/null
settle
expect_fail "$work/update-old-after.log" "arbitrator pin violated" "${P_OLD[@]}" update
echo "shipped profile fails closed on the wrapper: ok"
"$INTEND" "${P_NEW[@]}" update > "$work/update-new-after.log"; cat "$work/update-new-after.log"
[ "$(json_field "$work/update-new-after.log" arbitrator)" = "$(lower $W)" ] || fail "expected the wrapper arm"
echo "wrapper profile verifies through the wrapper: ok"

echo "== carry the install across and audit it =="
( cd "$work" && "$INTEND" "${P_NEW[@]}" migrate --from "$work/old.toml" --lockfile ./lock.json > migrate.log ); cat "$work/migrate.log"
python3 -I - "$work/migrate.log" <<'EOF'
import json, sys
r = json.loads(open(sys.argv[1]).read().strip().splitlines()[-1])
assert r["ok"] and r["migrated"] == 1 and r["states"]["current"] == 1, r
EOF
( cd "$work" && "$INTEND" "${P_NEW[@]}" audit --lockfile ./lock.json )

echo "== a submission after the switch pays the live deposit =="
LIVE=$(deposit submissionBaseDeposit)
BASE=$(num "$(cast call $REG "submissionBaseDeposit()(uint256)" --rpc-url $A)")
STALE=$(cast call $COURT "arbitrationCost(bytes)(uint256)" $ENVELOPE --rpc-url $A 2>/dev/null) \
  && STALE=$(python3 -I -c 'import sys; print(int(sys.argv[1]) + int(sys.argv[2]))' $BASE "$(num "$STALE")") \
  || STALE="a revert"
echo "deposit $LIVE wei from the live arbitrator; the court asked about the envelope would say $STALE"
[ "$LIVE" = "$(python3 -I -c 'import sys; print(int(sys.argv[1]) + int(sys.argv[2]))' $BASE $Q)" ] \
  || fail "the wrapper's quote is not the court cost"
ITEMS=$(num "$(cast call $REG "itemCount()(uint256)" --rpc-url $A)")
register "$(descriptor wrapper-era-skill "Submitted after the switch." "$CID")"
"$INTEND" "${P_NEW[@]}" update > "$work/update-new-submitted.log"; cat "$work/update-new-submitted.log"
[ "$(json_field "$work/update-new-submitted.log" items)" = $((ITEMS + 1)) ] || fail "the new item is not in the catalog"

echo "== any other arbitrator still fails closed: a second wrapper instance =="
W2=$(deploy_wrapper)
as_timelock $W2 "registerEpoch(bytes,uint256,uint256,uint256,string,string)" $EXTRA 0 0 $Q \
  "ipfs://fork-registration" "ipfs://fork-removal" > /dev/null
as_timelock $REG "changeArbitrator(address,bytes)" $W2 $ENVELOPE > /dev/null
settle
expect_fail "$work/update-w2.log" "arbitrator pin violated" "${P_NEW[@]}" update
( cd "$work" && expect_fail audit-w2.log "arbitrator pin violated" "${P_NEW[@]}" audit --lockfile ./lock.json )
echo "second instance $W2 fails closed: ok"
echo "WRAPPER FORK OK (work dir: $work)"
