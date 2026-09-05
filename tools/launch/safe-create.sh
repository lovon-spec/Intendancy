#!/usr/bin/env bash
# Prepare (and, on request, broadcast) the creation of a one-of-one Safe v1.4.1
# on Gnosis Chain through the canonical SafeProxyFactory, with the canonical
# SafeL2 singleton and CompatibilityFallbackHandler. Prints the predicted Safe
# address (a CREATE2 result, so it is known before broadcasting) and the exact
# `cast send` invocation. The key never passes through this script: with
# --broadcast, cast asks for it interactively.
#
#   safe-create.sh <owner-eoa> [--salt <uint>] [--rpc <url>] [--broadcast]
#
# After creation, run verify-safe.sh on the address before it goes anywhere.
set -euo pipefail
OWNER=${1:?usage: safe-create.sh <owner-eoa> [--salt <uint>] [--rpc <url>] [--broadcast]}; shift
SALT=$(date +%s); RPC=https://rpc.gnosischain.com; BROADCAST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --salt) SALT=$2; shift 2;;
    --rpc) RPC=$2; shift 2;;
    --broadcast) BROADCAST=1; shift;;
    *) echo "unknown argument $1" >&2; exit 2;;
  esac
done
# Canonical Safe v1.4.1 deployments (safe-global/safe-deployments); code hashes
# are checked live by verify-safe.sh.
FACTORY=0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67
SINGLETON=0x29fcB43b46531BcA003ddC8FCB67FFE91900C762   # SafeL2
HANDLER=0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99     # CompatibilityFallbackHandler
ZERO=0x0000000000000000000000000000000000000000
[ "$(cast chain-id --rpc-url "$RPC")" = "100" ] || { echo "RPC is not Gnosis Chain (chain id 100)" >&2; exit 1; }
INIT=$(cast calldata "setup(address[],uint256,address,bytes,address,address,uint256,address)" "[$OWNER]" 1 $ZERO 0x $HANDLER $ZERO 0 $ZERO)
PREDICTED=$(cast call $FACTORY "createProxyWithNonce(address,bytes,uint256)(address)" $SINGLETON "$INIT" "$SALT" --rpc-url "$RPC")
echo "owner:      $OWNER"
echo "salt:       $SALT"
echo "predicted:  $PREDICTED"
echo
echo "cast send $FACTORY 'createProxyWithNonce(address,bytes,uint256)' $SINGLETON $INIT $SALT --rpc-url $RPC --interactive"
if [ "$BROADCAST" = 1 ]; then
  echo; echo "broadcasting; cast will prompt for the key"
  cast send $FACTORY "createProxyWithNonce(address,bytes,uint256)" $SINGLETON "$INIT" "$SALT" --rpc-url "$RPC" --interactive
  echo "now run: verify-safe.sh $PREDICTED --owner $OWNER --rpc $RPC"
fi
