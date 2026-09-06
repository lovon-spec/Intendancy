#!/usr/bin/env bash
# Verify that an address is a freshly created one-of-one Safe v1.4.1 on Gnosis
# Chain before it becomes the registry's governor: canonical proxy code,
# canonical singleton and fallback handler (code hashes checked live against the
# safe-global/safe-deployments records), exactly one owner, threshold one, no
# guard, no modules. Fails closed on any deviation.
#
#   verify-safe.sh <safe-address> [--owner <expected-eoa>] [--rpc <url>]
set -uo pipefail
SAFE=${1:?usage: verify-safe.sh <safe-address> [--owner <expected-eoa>] [--rpc <url>]}; shift
OWNER=""; RPC=https://rpc.gnosischain.com
while [ $# -gt 0 ]; do
  case "$1" in
    --owner) OWNER=$2; shift 2;;
    --rpc) RPC=$2; shift 2;;
    *) echo "unknown argument $1" >&2; exit 2;;
  esac
done
# safe-global/safe-deployments v1.4.1, canonical addresses and code hashes.
SAFE_L2=0x29fcb43b46531bca003ddc8fcb67ffe91900c762;  SAFE_L2_HASH=0xb1f926978a0f44a2c0ec8fe822418ae969bd8c3f18d61e5103100339894f81ff
SAFE_L1=0x41675c099f32341bf84bfc5382af534df5c7461a;  SAFE_L1_HASH=0x1fe2df852ba3299d6534ef416eefa406e56ced995bca886ab7a553e6d0c5e1c4
HANDLER=0xfd0732dc9e303f09fcef3a7388ad10a83459ec99;  HANDLER_HASH=0x7c6007a5d711cea8dfd5d91f5940ec29c7f200fe511eb1fc1397b367af3c42f9
# keccak256 of the 171-byte SafeProxy v1.4.1 runtime, the tail of the factory's
# proxyCreationCode(); derived on a Gnosis fork (tools/launch/README.md).
PROXY_HASH=0xd7d408ebcd99b2b70be43e20253d6d92a8ea8fab29bd3be7f55b10032331fb4c
FALLBACK_SLOT=0x6c9a6c4a39284e37ed1cf53d337577d14212a4870fb976a4366c693b939918d5
GUARD_SLOT=0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8
ZERO32=0x0000000000000000000000000000000000000000000000000000000000000000

fails=0
ok()   { echo "  ok    $1"; }
bad()  { echo "  FAIL  $1"; fails=$((fails+1)); }
lower() { tr 'A-F' 'a-f' <<<"$1"; }
addr_of_word() { printf '0x%s' "${1: -40}"; }
codehash() { local c; c=$(cast code "$1" --rpc-url "$RPC" 2>/dev/null) || c=0x; [ "$c" = "0x" ] && echo none || cast keccak "$c"; }

SAFE=$(lower "$SAFE")
echo "Safe $SAFE via $RPC"
chain=$(cast chain-id --rpc-url "$RPC" 2>/dev/null); [ "$chain" = "100" ] && ok "chain id 100 (Gnosis)" || bad "chain id is '$chain', not 100"
[ "$(codehash "$SAFE")" = "$PROXY_HASH" ] && ok "proxy code is SafeProxy v1.4.1" || bad "code at the address is not the SafeProxy v1.4.1 runtime"
singleton=$(addr_of_word "$(cast storage "$SAFE" 0 --rpc-url "$RPC" 2>/dev/null)")
case "$singleton" in
  "$SAFE_L2") [ "$(codehash "$singleton")" = "$SAFE_L2_HASH" ] && ok "singleton is canonical SafeL2 v1.4.1 ($singleton)" || bad "SafeL2 singleton code hash changed";;
  "$SAFE_L1") [ "$(codehash "$singleton")" = "$SAFE_L1_HASH" ] && ok "singleton is canonical Safe v1.4.1 ($singleton)" || bad "Safe singleton code hash changed";;
  *) bad "singleton $singleton is not a canonical v1.4.1 singleton";;
esac
version=$(cast call "$SAFE" 'VERSION()(string)' --rpc-url "$RPC" 2>/dev/null); [ "$version" = '"1.4.1"' ] && ok "VERSION() 1.4.1" || bad "VERSION() returned $version"
owners=$(cast call "$SAFE" 'getOwners()(address[])' --rpc-url "$RPC" 2>/dev/null | tr -d '[] ' | tr ',' '\n' | grep -c .)
[ "$owners" = "1" ] && ok "exactly one owner" || bad "$owners owners"
owner=$(lower "$(cast call "$SAFE" 'getOwners()(address[])' --rpc-url "$RPC" 2>/dev/null | tr -d '[] ' | cut -d, -f1)")
echo "        owner $owner"
if [ -n "$OWNER" ]; then [ "$owner" = "$(lower "$OWNER")" ] && ok "owner matches the expected EOA" || bad "owner differs from the expected EOA $OWNER"; fi
threshold=$(cast call "$SAFE" 'getThreshold()(uint256)' --rpc-url "$RPC" 2>/dev/null); [ "$threshold" = "1" ] && ok "threshold 1" || bad "threshold $threshold"
fb=$(addr_of_word "$(cast storage "$SAFE" $FALLBACK_SLOT --rpc-url "$RPC" 2>/dev/null)")
if [ "$fb" = "$HANDLER" ]; then [ "$(codehash "$fb")" = "$HANDLER_HASH" ] && ok "fallback handler is the canonical CompatibilityFallbackHandler" || bad "fallback handler code hash changed"
elif [ "$fb" = "0x0000000000000000000000000000000000000000" ]; then ok "no fallback handler set (acceptable; the Safe web app would set the canonical one)"
else bad "fallback handler $fb is not canonical"; fi
guard=$(cast storage "$SAFE" $GUARD_SLOT --rpc-url "$RPC" 2>/dev/null); [ "$guard" = "$ZERO32" ] && ok "no guard" || bad "guard slot is $guard"
modules=$(cast call "$SAFE" 'getModulesPaginated(address,uint256)(address[],address)' 0x0000000000000000000000000000000000000001 10 --rpc-url "$RPC" 2>/dev/null | head -1); [ "$modules" = "[]" ] && ok "no modules" || bad "modules enabled: $modules"
nonce=$(cast call "$SAFE" 'nonce()(uint256)' --rpc-url "$RPC" 2>/dev/null); echo "        nonce $nonce"
if [ "$fails" = 0 ]; then echo "VERIFIED: $SAFE is a one-of-one Safe v1.4.1 owned by $owner"; else echo "NOT VERIFIED: $fails check(s) failed"; exit 1; fi
