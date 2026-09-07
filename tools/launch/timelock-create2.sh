#!/usr/bin/env bash
# Build the CREATE2 deployment of the governor timelock exactly as the Safe
# executed it on 2026-09-07, and print the address it lands on.
#
# A Safe cannot run CREATE itself, so it calls the standard CREATE2 deployer
# contract (0x4e59b44847b379578588920cA78FbF26c0B4956C, present on Gnosis and
# most other chains) with `salt ++ initcode` as raw calldata; the deployer runs
# CREATE2, so the resulting address depends only on the deployer, the salt and
# the initcode, never on who sends the transaction or when. The initcode is
# TimelockController's creation code from this repository's own build plus the
# constructor arguments: the delay, proposers = [the Safe], executors =
# [address(0)] for open execution, admin = address(0) so the timelock
# administers itself (the same arguments DeployTimelock.s.sol uses).
#
#   timelock-create2.sh <safe> [<out-dir>]
#
# Writes <out-dir>/initcode.hex and <out-dir>/calldata.hex (default: the current
# directory) and prints the runtime code hash (what verify-timelock.sh checks),
# the initcode hash, the CREATE2 address and the calldata keccak, which is what
# the owner compares in the Safe app before signing. TIMELOCK_MIN_DELAY_SECONDS
# overrides the delay (default 604800); CREATE2_DEPLOYER and CREATE2_SALT
# override the deployer and the 32-byte salt (default: all zero).
set -euo pipefail
SAFE=${1:?usage: timelock-create2.sh <safe> [<out-dir>]}
OUTDIR=${2:-.}
DELAY=${TIMELOCK_MIN_DELAY_SECONDS:-604800}
DEPLOYER=${CREATE2_DEPLOYER:-0x4e59b44847b379578588920cA78FbF26c0B4956C}
SALT=${CREATE2_SALT:-0000000000000000000000000000000000000000000000000000000000000000}
SALT=${SALT#0x}
[ ${#SALT} -eq 64 ] || { echo "CREATE2_SALT must be 32 bytes of hex" >&2; exit 2; }
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
command -v forge >/dev/null && command -v cast >/dev/null || { echo "foundry (forge and cast) is required" >&2; exit 2; }
mkdir -p "$OUTDIR"
# `forge inspect` wants the bare contract name; the path:Name form fails for a contract that lives in lib/.
CREATION=$(cd "$ROOT/contracts" && forge inspect --json TimelockController bytecode | tr -d '"\n')
RUNTIME=$(cd "$ROOT/contracts" && forge inspect --json TimelockController deployedBytecode | tr -d '"\n')
case "$CREATION" in 0x6080*) ;; *) echo "unexpected creation code from forge inspect: ${CREATION:0:20}" >&2; exit 1;; esac
ARGS=$(cast abi-encode 'constructor(uint256,address[],address[],address)' "$DELAY" "[$SAFE]" '[0x0000000000000000000000000000000000000000]' 0x0000000000000000000000000000000000000000)
INITCODE="${CREATION}${ARGS#0x}"
printf '%s' "$INITCODE" > "$OUTDIR/initcode.hex"
printf '0x%s%s' "$SALT" "${INITCODE#0x}" > "$OUTDIR/calldata.hex"
RUNTIME_HASH=$(cast keccak "$RUNTIME")
INIT_HASH=$(cast keccak "$INITCODE")
DIGEST=$(cast keccak "0xff${DEPLOYER#0x}${SALT}${INIT_HASH#0x}")
ADDRESS=$(cast to-check-sum-address "0x${DIGEST: -40}")
CALLDATA_HASH=$(cast keccak "$(cat "$OUTDIR/calldata.hex")")
echo "safe (proposer and canceller): $SAFE"
echo "delay (seconds):               $DELAY"
echo "create2 deployer:              $DEPLOYER"
echo "salt:                          0x$SALT"
echo "runtime code hash:             $RUNTIME_HASH"
echo "initcode hash:                 $INIT_HASH ($(( (${#INITCODE} - 2) / 2 )) bytes, $OUTDIR/initcode.hex)"
echo "timelock address (CREATE2):    $ADDRESS"
echo "calldata keccak:               $CALLDATA_HASH ($OUTDIR/calldata.hex)"
echo "Safe transaction: to = the create2 deployer, value 0, operation Call, data = calldata.hex; see propose-safe-tx.sh"
