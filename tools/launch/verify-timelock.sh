#!/usr/bin/env bash
# Verify that an address is the registry governor we intend: a self-administered
# OpenZeppelin TimelockController with at least a seven-day delay, the Safe as
# proposer and canceller, open execution, and no admin held by the Safe or the
# deployer. Fails closed on any deviation. Run it before the address goes into
# GOVERNOR, and again after the registry is deployed.
#
#   verify-timelock.sh <timelock> --proposer <safe> [--deployer <eoa>] [--rpc <url>] [--min-delay <seconds>]
#
# The expected runtime code hash is computed from this repository's build of
# contracts/lib/openzeppelin-contracts (v5.7.0) with the repository's compiler
# settings; a timelock deployed from another build fails the code-hash check
# and must be inspected by hand.
set -uo pipefail
TL=${1:?usage: verify-timelock.sh <timelock> --proposer <safe> [--deployer <eoa>] [--rpc <url>] [--min-delay <seconds>]}; shift
PROPOSER=""; DEPLOYER=""; RPC=https://rpc.gnosischain.com; MIN_DELAY=604800
while [ $# -gt 0 ]; do
  case "$1" in
    --proposer) PROPOSER=$2; shift 2;;
    --deployer) DEPLOYER=$2; shift 2;;
    --rpc) RPC=$2; shift 2;;
    --min-delay) MIN_DELAY=$2; shift 2;;
    *) echo "unknown argument $1" >&2; exit 2;;
  esac
done
[ -n "$PROPOSER" ] || { echo "--proposer <safe> is required" >&2; exit 2; }
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(cd "$HERE/../.." && pwd)
PROPOSER_ROLE=$(cast keccak "PROPOSER_ROLE"); EXECUTOR_ROLE=$(cast keccak "EXECUTOR_ROLE"); CANCELLER_ROLE=$(cast keccak "CANCELLER_ROLE")
ADMIN_ROLE=0x0000000000000000000000000000000000000000000000000000000000000000
ZERO=0x0000000000000000000000000000000000000000
fails=0
ok()  { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fails=$((fails+1)); }
lower() { tr 'A-F' 'a-f' <<<"$1"; }
has_role() { cast call "$TL" 'hasRole(bytes32,address)(bool)' "$1" "$2" --rpc-url "$RPC" 2>/dev/null; }

TL=$(lower "$TL"); PROPOSER=$(lower "$PROPOSER")
echo "Timelock $TL via $RPC"
code=$(cast code "$TL" --rpc-url "$RPC" 2>/dev/null || echo 0x)
[ "$code" != "0x" ] && ok "address holds code" || bad "no code at the address"
expected=$(cd "$ROOT/contracts" && forge inspect --json TimelockController deployedBytecode 2>/dev/null | tr -d '"')
if [ -n "$expected" ] && [ "$expected" != "null" ]; then
  [ "$(cast keccak "$code")" = "$(cast keccak "$expected")" ] && ok "runtime code matches the repository's TimelockController build" || bad "runtime code differs from the repository's TimelockController build"
else
  bad "could not compute the expected code hash (run forge build in contracts/)"
fi
delay=$(cast call "$TL" 'getMinDelay()(uint256)' --rpc-url "$RPC" 2>/dev/null | awk '{print $1}')
[ -n "$delay" ] && [ "$delay" -ge "$MIN_DELAY" ] 2>/dev/null && ok "minimum delay $delay s (at least $MIN_DELAY)" || bad "minimum delay is '$delay', below $MIN_DELAY"
[ "$(has_role $PROPOSER_ROLE "$PROPOSER")" = "true" ] && ok "proposer role held by $PROPOSER" || bad "proposer role not held by $PROPOSER"
[ "$(has_role $CANCELLER_ROLE "$PROPOSER")" = "true" ] && ok "canceller role held by the proposer" || bad "canceller role not held by the proposer"
[ "$(has_role $EXECUTOR_ROLE $ZERO)" = "true" ] && ok "execution is open" || bad "execution is not open"
[ "$(has_role $ADMIN_ROLE "$TL")" = "true" ] && ok "the timelock administers itself" || bad "the timelock does not administer itself"
[ "$(has_role $ADMIN_ROLE "$PROPOSER")" = "false" ] && ok "the proposer holds no admin role" || bad "the proposer holds the admin role"
if [ -n "$DEPLOYER" ]; then
  [ "$(has_role $ADMIN_ROLE "$(lower "$DEPLOYER")")" = "false" ] && ok "the deployer holds no admin role" || bad "the deployer holds the admin role"
  [ "$(has_role $PROPOSER_ROLE "$(lower "$DEPLOYER")")" = "false" ] && ok "the deployer holds no proposer role" || bad "the deployer holds the proposer role"
fi
if [ "$fails" = 0 ]; then echo "VERIFIED: $TL is a self-administered timelock, delay $delay s, proposer $PROPOSER"; else echo "NOT VERIFIED: $fails check(s) failed"; exit 1; fi
