#!/bin/sh
# Authenticate a deployed Intendment wrapper (IntendmentArbitrator) and print the
# [arbitrator_wrapper] table a profile needs to accept it. Nothing is printed until its
# provenance is proven:
#   1. the intendment checkout at $INTENDMENT is clean and at the commit pinned in
#      wrapper-provenance.env, and that commit is built with the repository's own foundry.toml;
#   2. the factory's runtime code hash is the build's IntendmentFactory;
#   3. the wrapper is that factory's CREATE2 deployment n for the profile's registry and
#      governor, computed here from the build's IntendmentArbitrator creation code and the
#      constructor arguments (host the registry, resolver the arbitrator, governor the governor,
#      host profile and config from wrapper-provenance.env); it has code, and the factory's
#      predict() agrees.
# Only the registry's governor can deploy at that address, so its code is the reviewed wrapper
# with exactly those immutables, and its runtime code hash is safe to pin. The getter and
# arbitrationCost checks that follow are sanity checks, not the trust anchor: a lookalike answers
# them however it likes.
#
# Read-only: it sends no transaction. Run it against Gnosis once the wrapper exists, or against a
# fork that executed the governance batch (tools/wrapper-fork.sh does).
#
# usage: INTENDMENT=<intendment checkout> pin-wrapper.sh <rpc-url> <factory> <wrapper> [profile.toml]
#        INTENDMENT=<intendment checkout> pin-wrapper.sh --build   # build only; prints the artifacts dir
# PIN_WORK=<dir> keeps the build between runs (default: a new temporary directory).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
fail() { echo "pin-wrapper: $*" >&2; exit 1; }
lower() { echo "$1" | tr 'A-F' 'a-f'; }
same() { [ "$(lower "$1")" = "$(lower "$2")" ]; }
pinned() { # <key> <extended regex the value must match>
  v=$(sed -n "s/^$1=\([^ #]*\).*/\1/p" "$here/wrapper-provenance.env")
  echo "$v" | grep -Eqx "$2" || fail "wrapper-provenance.env: $1 is missing or malformed"
  echo "$v"
}
COMMIT=$(pinned INTENDMENT_COMMIT '[0-9a-f]{40}')
HOST_PROFILE=$(pinned HOST_PROFILE '[01]')
CONFIG=$(pinned CONFIG '[0-9]+(,[0-9]+){4}')
DEPLOYMENT=$(pinned DEPLOYMENT '[0-9]+')
EPOCH=$(pinned EPOCH '[1-9][0-9]*')
CHECKOUT=${INTENDMENT:?set INTENDMENT to an intendment checkout at the pinned commit}

# 1. The pinned source, built from exactly the commit's tree (the checkout itself is not written).
[ "$(git -C "$CHECKOUT" rev-parse HEAD)" = "$COMMIT" ] || fail "$CHECKOUT is not at the pinned commit $COMMIT"
[ -z "$(git -C "$CHECKOUT" status --porcelain --untracked-files=all)" ] || fail "$CHECKOUT has uncommitted changes"
WORK=${PIN_WORK:-$(mktemp -d)}
SRC="$WORK/intendment-$COMMIT"
if [ ! -f "$SRC/.built" ]; then
  rm -rf "$SRC" && mkdir -p "$SRC"
  git -C "$CHECKOUT" archive -o "$SRC.tar" "$COMMIT"
  tar -x -f "$SRC.tar" -C "$SRC"
  (cd "$SRC" && forge build) > "$SRC.log" 2>&1 || fail "forge build of intendment $COMMIT failed (see $SRC.log)"
  touch "$SRC/.built"
fi
OUT="$SRC/contracts/out"
[ "${1:-}" = --build ] && { echo "$OUT"; exit 0; }

RPC=${1:?usage: pin-wrapper.sh <rpc-url> <factory> <wrapper> [profile.toml]}
FACTORY=${2:?usage: pin-wrapper.sh <rpc-url> <factory> <wrapper> [profile.toml]}
W=${3:?usage: pin-wrapper.sh <rpc-url> <factory> <wrapper> [profile.toml]}
PROFILE=${4:-$here/../profiles/agent-skills-registry.toml}
for a in "$FACTORY" "$W"; do echo "$a" | grep -Eqx '0x[0-9a-fA-F]{40}' || fail "$a is not an address"; done
pin() { python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))[sys.argv[2]])' "$PROFILE" "$1"; }
artifact() { python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]]["object"])' "$OUT/$1.sol/$1.json" "$2"; }
REG=$(pin registry); ARB=$(pin arbitrator); GOV=$(pin governor); EXTRA=$(pin arbitrator_extra_data)

# 2. The factory runs the build's code.
FACTORY_HASH=$(cast keccak "$(artifact IntendmentFactory deployedBytecode)")
ONCHAIN=$(cast codehash "$FACTORY" --rpc-url "$RPC")
same "$ONCHAIN" "$FACTORY_HASH" \
  || fail "factory $FACTORY runs code hashing to $ONCHAIN, not intendment $COMMIT's IntendmentFactory ($FACTORY_HASH)"

# 3. The wrapper is that factory's CREATE2 deployment n for the registry and its governor.
SIG='(address,address,uint256,address,uint8,(uint256,uint256,uint256,uint256,uint256))'
ARGS=$(cast abi-encode "f(address,address,address,uint8,(uint256,uint256,uint256,uint256,uint256))" \
  "$REG" "$ARB" "$GOV" "$HOST_PROFILE" "($CONFIG)")
INIT_HASH=$(cast keccak "$(artifact IntendmentArbitrator bytecode)${ARGS#0x}")
SALT=$(cast keccak "$(cast abi-encode "f(address,address,uint256)" "$REG" "$GOV" "$DEPLOYMENT")")
EXPECTED=0x$(cast keccak "0xff${FACTORY#0x}${SALT#0x}${INIT_HASH#0x}" | cut -c27-66)
same "$W" "$EXPECTED" \
  || fail "$W is not $FACTORY's CREATE2 deployment $DEPLOYMENT for $REG by $GOV, which is $EXPECTED"
PREDICTED=$(cast call "$FACTORY" "predict$SIG(address)" "$REG" "$GOV" "$DEPLOYMENT" "$ARB" "$HOST_PROFILE" \
  "($CONFIG)" --rpc-url "$RPC")
same "$PREDICTED" "$EXPECTED" || fail "the factory predicts $PREDICTED, computed $EXPECTED"
[ "$(cast codesize "$W" --rpc-url "$RPC")" -gt 0 ] || fail "$W has no code yet"

# Sanity: the immutables and the epoch answer as expected (not the trust anchor; see above).
getter() { cast call "$W" "$1" --rpc-url "$RPC" 2>/dev/null || fail "$W answers no $1"; }
same "$(getter "HOST()(address)")" "$REG" || fail "HOST() is not the registry $REG"
same "$(getter "K()(address)")" "$ARB" || fail "K() is not the arbitrator $ARB"
same "$(getter "GOVERNOR()(address)")" "$GOV" || fail "GOVERNOR() is not the governor $GOV"
same "$(getter "FACTORY()(address)")" "$FACTORY" || fail "FACTORY() is not $FACTORY"
[ "$(getter "PROFILE()(uint8)")" = "$HOST_PROFILE" ] || fail "PROFILE() is not $HOST_PROFILE"
ENVELOPE=$(cast abi-encode "f(uint8,uint32,bytes)" 1 "$EPOCH" "$EXTRA")
QUOTE=$(cast call "$W" "arbitrationCost(bytes)(uint256)" "$ENVELOPE" --rpc-url "$RPC" 2>/dev/null) \
  || fail "epoch $EPOCH does not exist or does not forward the pinned court data $EXTRA"
COURT=$(cast call "$ARB" "arbitrationCost(bytes)(uint256)" "$EXTRA" --rpc-url "$RPC")
NOW=$(cast call "$REG" "arbitrator()(address)" --rpc-url "$RPC")
if same "$NOW" "$W"; then
  same "$(cast call "$REG" "arbitratorExtraData()(bytes)" --rpc-url "$RPC")" "$ENVELOPE" \
    || fail "the registry names $W with extra data other than the epoch $EPOCH envelope"
fi

CODE_HASH=$(cast codehash "$W" --rpc-url "$RPC")
echo "pin-wrapper: authenticated against intendment $COMMIT: factory $FACTORY runs its IntendmentFactory and $W is its deployment $DEPLOYMENT for $REG by $GOV; epoch $EPOCH quotes ${QUOTE%% *} wei, the court ${COURT%% *} wei" >&2
cat <<EOF
# Authenticated by cli/tools/pin-wrapper.sh against intendment $COMMIT: deployment $DEPLOYMENT of
# IntendmentFactory $FACTORY (runtime $FACTORY_HASH).
[arbitrator_wrapper]
address = "$(cast to-check-sum-address "$W")"
code_hash = "$CODE_HASH"
epoch = $EPOCH
EOF
