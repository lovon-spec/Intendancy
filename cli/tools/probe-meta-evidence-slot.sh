#!/bin/zsh
# ASSERTING reproduction of docs/spikes/meta-evidence-slot-evidence.md: deploys
# a registry via the REAL GTCRFactory on an anvil Gnosis fork, then PROVES (or
# fails loudly) each pinned claim:
#   A. the deployed runtime codehash equals the pinned constant, recomputed
#      locally from eth_getCode;
#   B. metaEvidenceUpdates() reads 0 at deployment;
#   C. a governor changeMetaEvidence moves the getter 0→1 and storage slot 9
#      0→1 while every other SAMPLED slot (0..16) is byte-identical;
#   D. a second changeMetaEvidence moves getter and slot 9 to 2 (the observed
#      transition is an increment);
#   E. layout cross-checks: slot 0 = arbitrator, slot 13 = itemList length;
#   F. the LOAD-BEARING no-reset property's source evidence: the verified
#      GeneralizedTCR source of the same-codehash mainnet canary
#      (0x54A92C21…6610) is re-retrieved from Blockscout; asserted: sha256
#      equals the evidence-record pin, verification fields (name, compiler,
#      is_verified, is_partially_verified, is_changed_bytecode == false),
#      and the source-text occurrence counts (one `metaEvidenceUpdates++`
#      write; zero assembly/delegatecall/selfdestruct). The retrieved source
#      is left in $work for inspection.
#   G. the same property at the BYTECODE level (source text cannot exclude
#      compiler-emitted delegation): the canary's exact on-fork runtime is
#      disassembled — Solidity CBOR metadata stripped by its trailing length
#      and validated as a CBOR map, PUSH immediates skipped — and asserted to
#      contain NO executable DELEGATECALL (0xf4), CALLCODE (0xf2), or
#      SELFDESTRUCT (0xff) opcode.
# Any violated assertion exits nonzero. The ASSERTION TRANSCRIPT (registry,
# codehashes, both tx receipts with hashes/blocks, every sampled slot read,
# the source retrieval and disassembly summary — a summary of asserted
# values, not a raw byte capture) is written to $work/transcript.txt, its
# sha256 printed, and — if PROBE_OUT is set — copied there.
# Requires anvil, cast, python3, curl, and the Gate 2 spike crate.
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/.." && pwd)"
work="$(mktemp -d)"
RPC=http://127.0.0.1:8547
KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
PINNED_CODEHASH=0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d
ARBITRATOR=0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002
CANARY=0x54A92C21c6553a8085066311F2C8D9Db1B5e6610
SOURCE_URL="https://gnosis.blockscout.com/api/v2/smart-contracts/$CANARY"
PINNED_SOURCE_SHA256=2cf70f05773971382aa44e2ec5e9752f4e23a0edbaef1b1cb5230bc6c707e0d3
T="$work/transcript.txt"

fail() { echo "FAIL: $*" | tee -a "$T" >&2; exit 1; }
log()  { echo "$*" | tee -a "$T"; }

# F. Load-bearing source pin FIRST (independent of the fork): re-retrieve the
#    verified source and assert its hash. Command recorded verbatim:
#    curl -sL "$SOURCE_URL" | python3 -c 'json.load(stdin)["source_code"]'
log "source retrieval: curl -sL $SOURCE_URL"
curl -sL "$SOURCE_URL" -o "$work/canary-contract.json" || fail "source retrieval failed"
python3 - "$work/canary-contract.json" "$work/canary-source.sol" <<'EOF' >> "$T" || fail "verified source extraction/field assertions failed"
import json, sys
d = json.load(open(sys.argv[1]))
src = d.get("source_code") or ""
assert d.get("name") == "GeneralizedTCR", d.get("name")
assert d.get("compiler_version") == "v0.5.17+commit.d19bba13", d.get("compiler_version")
assert d.get("is_verified") is True, d.get("is_verified")
assert d.get("is_partially_verified") is True, d.get("is_partially_verified")
assert d.get("is_changed_bytecode") is False, d.get("is_changed_bytecode")
assert src, "empty source_code"
open(sys.argv[2], "w").write(src)
print(f"verified source: name={d.get('name')} compiler={d.get('compiler_version')} "
      f"verified={d.get('is_verified')} partially={d.get('is_partially_verified')} "
      f"changed_bytecode={d.get('is_changed_bytecode')} bytes={len(src.encode())}")
EOF
GOT_SOURCE_SHA=$(shasum -a 256 "$work/canary-source.sol" | cut -d' ' -f1)
log "source sha256: $GOT_SOURCE_SHA (pinned: $PINNED_SOURCE_SHA256)"
[ "$GOT_SOURCE_SHA" = "$PINNED_SOURCE_SHA256" ] || fail "canary source hash != evidence-record pin"
WRITES=$(grep -c "metaEvidenceUpdates++" "$work/canary-source.sol" || true)
OCCURRENCES=$(grep -c "metaEvidenceUpdates" "$work/canary-source.sol" || true)
ASSEMBLY=$(grep -c "assembly" "$work/canary-source.sol" || true)
DELEGATE=$(grep -c "delegatecall" "$work/canary-source.sol" || true)
SELFDESTRUCT=$(grep -c "selfdestruct" "$work/canary-source.sol" || true)
log "source occurrences: $OCCURRENCES total, $WRITES increment write(s), $ASSEMBLY assembly, $DELEGATE delegatecall, $SELFDESTRUCT selfdestruct"
[ "$WRITES" = "1" ] || fail "expected exactly one metaEvidenceUpdates++ write, got $WRITES"
[ "$OCCURRENCES" = "6" ] || fail "expected six metaEvidenceUpdates occurrences, got $OCCURRENCES"
[ "$ASSEMBLY" = "0" ] || fail "expected zero assembly blocks, got $ASSEMBLY"
[ "$DELEGATE" = "0" ] || fail "expected zero delegatecall occurrences, got $DELEGATE"
[ "$SELFDESTRUCT" = "0" ] || fail "expected zero selfdestruct occurrences, got $SELFDESTRUCT"

anvil --fork-url https://rpc.gnosischain.com --fork-block-number 47881774 \
      --port 8547 --slots-in-an-epoch 1 > "$work/anvil.log" 2>&1 &
ANVIL=$!
trap 'kill $ANVIL 2>/dev/null || true' EXIT
sleep 7

( cd "$repo/spikes/snapshot-bench" && \
  cargo run --release --locked -q -- seed --rpc-url $RPC --items 3 \
    --out "$work/manifest.json" > /dev/null )
REG=$(python3 -c "import json;print(json.load(open('$work/manifest.json'))['registry'])")
MANIFEST_CODEHASH=$(python3 -c "import json;print(json.load(open('$work/manifest.json'))['registryCodeHash'])")
log "probe date: $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
log "fork: gnosis @ 47881774 (anvil --slots-in-an-epoch 1)"
log "registry:  $REG"

# A. codehash: manifest value, RECOMPUTED locally from the served bytecode,
#    and the pinned constant must all agree.
CODE=$(cast code $REG --rpc-url $RPC)
RECOMPUTED=$(cast keccak "$CODE")
log "codehash (manifest):   $MANIFEST_CODEHASH"
log "codehash (recomputed): $RECOMPUTED"
[ "$RECOMPUTED" = "$PINNED_CODEHASH" ] || fail "recomputed codehash != pinned constant"
[ "$MANIFEST_CODEHASH" = "$PINNED_CODEHASH" ] || fail "manifest codehash != pinned constant"

# Close the source->canary->reviewed-bytecode chain IN THIS repro: the mainnet
# canary whose verified source is pinned above must carry the SAME runtime
# codehash as the freshly deployed registry (the fork serves mainnet state).
cast code $CANARY --rpc-url $RPC > "$work/canary-code.hex"
CANARY_HASH=$(cast keccak "$(cat "$work/canary-code.hex")")
log "canary runtime codehash: $CANARY_HASH"
[ "$CANARY_HASH" = "$PINNED_CODEHASH" ] || fail "canary runtime codehash != pinned constant"

# G. BYTECODE-level no-reset evidence: strip the Solidity CBOR metadata
#    (validated), disassemble skipping PUSH immediates, and assert no
#    executable DELEGATECALL/CALLCODE/SELFDESTRUCT opcode exists.
python3 - "$work/canary-code.hex" <<'EOF' >> "$T" || fail "bytecode disassembly assertions failed"
import sys
code = bytes.fromhex(open(sys.argv[1]).read().strip()[2:])
mlen = int.from_bytes(code[-2:], "big")
assert mlen + 2 < len(code), f"implausible metadata length {mlen}"
cbor = code[-(mlen + 2):-2]
# FULLY parse the expected solc 0.5.17 metadata map — the executable boundary
# claim is only as good as this parse (round-7): map(2) {"bzzr1": bytes(32),
# "solc": bytes(3) == 00 05 11}, consumed exactly, total length exactly 50.
assert mlen == 50, f"metadata length {mlen} != expected 50 for solc 0.5.17"
p = 0
assert cbor[p] == 0xA2, f"not a two-entry CBOR map: {cbor[0]:#x}"; p += 1
assert cbor[p] == 0x65, "key 1 is not text(5)"; p += 1
assert cbor[p:p+5] == b"bzzr1", f"key 1 is {cbor[p:p+5]!r}, not bzzr1"; p += 5
assert cbor[p] == 0x58 and cbor[p+1] == 0x20, "bzzr1 value is not bytes(32)"; p += 2
p += 32  # the swarm hash itself varies per source; length is what matters
assert cbor[p] == 0x64, "key 2 is not text(4)"; p += 1
assert cbor[p:p+4] == b"solc", f"key 2 is {cbor[p:p+4]!r}, not solc"; p += 4
assert cbor[p] == 0x43, "solc value is not bytes(3)"; p += 1
assert cbor[p:p+3] == bytes([0x00, 0x05, 0x11]), \
    f"solc version bytes {cbor[p:p+3].hex()} != 000511 (0.5.17)"; p += 3
assert p == len(cbor), f"CBOR not fully consumed: {p} of {len(cbor)}"
body = code[:-(mlen + 2)]
flagged = []
pc = 0
while pc < len(body):
    op = body[pc]
    if 0x60 <= op <= 0x7F:  # PUSH1..PUSH32: skip immediates
        imm = op - 0x5F
        assert pc + 1 + imm <= len(body), \
            f"truncated PUSH immediate at {pc} — executable boundary is wrong"
        pc += 1 + imm
        continue
    if op in (0xF2, 0xF4, 0xFF):  # CALLCODE, DELEGATECALL, SELFDESTRUCT
        flagged.append((pc, hex(op)))
    pc += 1
assert not flagged, f"forbidden executable opcodes at {flagged}"
print(f"bytecode: runtime {len(code)} B; CBOR metadata FULLY parsed (50 B map: "
      f"bzzr1 bytes32 + solc 000511, consumed exactly); executable body {len(body)} B "
      f"disassembled (no truncated PUSH) — zero DELEGATECALL/CALLCODE/SELFDESTRUCT")
EOF

read_slots() { for i in $(seq 0 16); do echo "slot $i: $(cast storage $REG $i --rpc-url $RPC)"; done }
getter() { cast call $REG 'metaEvidenceUpdates()(uint256)' --rpc-url $RPC; }

# B. pristine deployment: getter must read 0.
G0=$(getter)
log "getter before: $G0"
[ "$G0" = "0" ] || fail "metaEvidenceUpdates != 0 at deployment: $G0"
read_slots > "$work/before.txt"
cat "$work/before.txt" >> "$T"

# E. layout cross-checks against known deploy parameters.
S0=$(grep '^slot 0:' "$work/before.txt" | awk '{print $3}')
S13=$(grep '^slot 13:' "$work/before.txt" | awk '{print $3}')
python3 - "$S0" "$ARBITRATOR" <<'EOF' || fail "slot 0 is not the arbitrator"
import sys; assert int(sys.argv[1],16) == int(sys.argv[2],16), (sys.argv[1], sys.argv[2])
EOF
[ $((S13)) -eq 3 ] || fail "slot 13 (itemList length) != 3: $S13"
log "layout cross-checks: slot 0 = arbitrator, slot 13 = 3 items — OK"

# C. governor changeMetaEvidence #1: getter and slot 9 move 0→1 together;
#    every other slot 0..16 is untouched.
cast send $REG "changeMetaEvidence(string,string)" "/ipfs/new-reg.json" "/ipfs/new-clr.json" \
  --private-key $KEY --rpc-url $RPC --json > "$work/tx1.json"
python3 - "$work/tx1.json" <<'EOF' >> "$T" || fail "changeMetaEvidence #1 did not succeed"
import json,sys
r = json.load(open(sys.argv[1]))
assert r["status"] in ("0x1", 1), r["status"]
print(f"tx1: hash {r['transactionHash']} block {int(r['blockNumber'],16) if isinstance(r['blockNumber'],str) else r['blockNumber']} status {r['status']}")
EOF
G1=$(getter)
log "getter after #1: $G1"
[ "$G1" = "1" ] || fail "getter != 1 after first changeMetaEvidence: $G1"
read_slots > "$work/after1.txt"
cat "$work/after1.txt" >> "$T"
for i in $(seq 0 16); do
  b=$(grep "^slot $i:" "$work/before.txt" | awk '{print $3}')
  a=$(grep "^slot $i:" "$work/after1.txt" | awk '{print $3}')
  if [ "$i" -eq 9 ]; then
    [ $((b)) -eq 0 ] || fail "slot 9 nonzero before: $b"
    [ $((a)) -eq 1 ] || fail "slot 9 != 1 after: $a"
  else
    [ "$b" = "$a" ] || fail "slot $i changed unexpectedly: $b -> $a"
  fi
done
log "assertion: getter 0->1 in lockstep with slot 9 alone; every other SAMPLED slot (0..16) unchanged — OK"

# D. changeMetaEvidence #2: the transition is an INCREMENT (1→2), matching the
#    verified source's `metaEvidenceUpdates++` (no reset path).
cast send $REG "changeMetaEvidence(string,string)" "/ipfs/new-reg-2.json" "/ipfs/new-clr-2.json" \
  --private-key $KEY --rpc-url $RPC --json > "$work/tx2.json"
python3 - "$work/tx2.json" <<'EOF' >> "$T" || fail "changeMetaEvidence #2 did not succeed"
import json,sys
r = json.load(open(sys.argv[1]))
assert r["status"] in ("0x1", 1), r["status"]
print(f"tx2: hash {r['transactionHash']} block {int(r['blockNumber'],16) if isinstance(r['blockNumber'],str) else r['blockNumber']} status {r['status']}")
EOF
G2=$(getter)
S9_2=$(cast storage $REG 9 --rpc-url $RPC)
log "getter after #2: $G2; slot 9: $S9_2"
[ "$G2" = "2" ] || fail "getter != 2 after second changeMetaEvidence: $G2"
[ $((S9_2)) -eq 2 ] || fail "slot 9 != 2 after second changeMetaEvidence: $S9_2"

log "ALL ASSERTIONS PASSED (transcript: $T)"
echo "transcript sha256: $(shasum -a 256 "$T" | cut -d' ' -f1)"
if [ -n "${PROBE_OUT:-}" ]; then cp "$T" "$PROBE_OUT"; echo "transcript copied to $PROBE_OUT"; fi
