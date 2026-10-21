#!/bin/sh
# Check a deployed Intendment wrapper (IntendmentArbitrator) against a profile and print the
# [arbitrator_wrapper] table that profile needs to accept it. Fails unless HOST() is the
# profile's registry, K() its arbitrator, GOVERNOR() its governor and PROFILE() Classic, and
# unless the wrapper accepts the envelope abi.encode(uint8(1), uint32(epoch),
# arbitrator_extra_data), which it does only when that epoch forwards exactly the pinned court
# data. The code hash it prints is what the CLI then proves at every anchor; those immutables
# are part of the code it hashes.
#
# Run it against Gnosis once the wrapper is deployed, or against a fork that executed the
# governance batch (tools/wrapper-fork.sh does that). Read-only: it sends no transaction.
#
# usage: pin-wrapper.sh <rpc-url> <wrapper> [epoch] [profile.toml]
set -eu
RPC=${1:?usage: pin-wrapper.sh <rpc-url> <wrapper> [epoch] [profile.toml]}
W=${2:?usage: pin-wrapper.sh <rpc-url> <wrapper> [epoch] [profile.toml]}
EPOCH=${3:-1}
PROFILE=${4:-$(dirname "$0")/../profiles/agent-skills-registry.toml}
fail() { echo "pin-wrapper: $*" >&2; exit 1; }
pin() { python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))[sys.argv[2]])' "$PROFILE" "$1"; }
lower() { echo "$1" | tr 'A-F' 'a-f'; }
same() { [ "$(lower "$1")" = "$(lower "$2")" ]; }

REG=$(pin registry); ARB=$(pin arbitrator); GOV=$(pin governor); EXTRA=$(pin arbitrator_extra_data)
CODE_HASH=$(cast codehash "$W" --rpc-url "$RPC")
case "$CODE_HASH" in
  0x0000000000000000000000000000000000000000000000000000000000000000|0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470)
    fail "$W has no code";;
esac
getter() { cast call "$W" "$1" --rpc-url "$RPC" 2>/dev/null || fail "$W answers no $1: not an IntendmentArbitrator"; }
HOST=$(getter "HOST()(address)"); K=$(getter "K()(address)")
WGOV=$(getter "GOVERNOR()(address)"); KIND=$(getter "PROFILE()(uint8)")
same "$HOST" "$REG" || fail "HOST() is $HOST, not the registry $REG"
same "$K" "$ARB" || fail "K() is $K, not the arbitrator $ARB"
same "$WGOV" "$GOV" || fail "GOVERNOR() is $WGOV, not the governor $GOV"
[ "$KIND" = 1 ] || fail "PROFILE() is $KIND, not Classic (1)"
ENVELOPE=$(cast abi-encode "f(uint8,uint32,bytes)" 1 "$EPOCH" "$EXTRA")
QUOTE=$(cast call "$W" "arbitrationCost(bytes)(uint256)" "$ENVELOPE" --rpc-url "$RPC" 2>/dev/null) \
  || fail "epoch $EPOCH does not exist or does not forward the pinned court data $EXTRA"
COURT=$(cast call "$ARB" "arbitrationCost(bytes)(uint256)" "$EXTRA" --rpc-url "$RPC")
echo "pin-wrapper: HOST, K, GOVERNOR and PROFILE match; epoch $EPOCH quotes ${QUOTE%% *} wei, the court ${COURT%% *} wei" >&2
NOW=$(cast call "$REG" "arbitrator()(address)" --rpc-url "$RPC")
if same "$NOW" "$W"; then
  same "$(cast call "$REG" "arbitratorExtraData()(bytes)" --rpc-url "$RPC")" "$ENVELOPE" \
    || fail "the registry names $W with extra data other than the epoch $EPOCH envelope"
  echo "pin-wrapper: the registry already names this wrapper with this envelope" >&2
fi
cat <<EOF
[arbitrator_wrapper]
address = "$(cast to-check-sum-address "$W")"
code_hash = "$CODE_HASH"
epoch = $EPOCH
EOF
