# Storage-slot evidence: `metaEvidenceUpdates` = slot 9

**Status**: evidence record for the PROVISIONAL slot-9 constant in
`verified-snapshot-spec.md` §5 — freezes when this record is reviewer-accepted.
**Date**: 2026-08-24, resealed 2026-08-25 (asserting run `2026-08-25T03:03:22Z`).
**Method**: the Gate 1 methodology — pin by observed on-chain behavior against
the exact codehash — plus, because the verifier's soundness needs a property
behavior alone cannot show (see Semantics), a verified-source pin for the
no-reset property.
**Companion artifacts** (review note): the probe script referenced below and
the verifier that ENFORCES this invariant live in the `intend` CLI
(`cli/tools/probe-meta-evidence-slot.sh`; `cli/src/snapshot.rs` §6 step 3b
plus the per-install point check), which lands in the companion CLI pull
request; the spec text defining step 3b lands in the companion docs pull
request. The Gate 2 spike crate in THIS change set predates the slot-9 owner
decision and deliberately proves only the item-list length, entries, and
statuses — it does not and is not claimed to prove slot 9. Until all three
companion changes merge, these references resolve only in the combined tree.
**Reproduction**: `cli/tools/probe-meta-evidence-slot.sh` — every claim below,
the verified-source hash and bytecode disassembly included, is an ASSERTION
in the script (any violation exits nonzero). The ASSERTION TRANSCRIPT — a
summary of every asserted value and receipt, not a raw byte capture — is
sealed at
`docs/spikes/meta-evidence-slot-transcript.txt`, sha256
`6f474bfcdca306060ae3f5c3e5b5ea42c3425770a085d31ef08810187856e454`.

## Environment

anvil 1.5.1-stable, fork of Gnosis (`--fork-url https://rpc.gnosischain.com
--fork-block-number 47881774 --slots-in-an-epoch 1`). Registry deployed through
the REAL `GTCRFactory` (`0x794Cee5a6e1501b633eC13b8c1e327d9860FE039`) by the
Gate 2 spike seeder (`spikes/snapshot-bench`, `seed --items 3`):

| | |
|---|---|
| Registry | `0x2a25ec70329918d13ec23477716a159d063735d0` |
| Runtime codehash | `0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d`, asserted equal between the seed manifest, a LOCAL recomputation `keccak256(eth_getCode(registry))`, and the pinned constant (the generation shared by the Gate 1 canary and both Gate 2 deployments) |
| Governor | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` (anvil account 0, per the seeder's deploy parameters) |

## Probe (all steps asserted, receipts in the sealed transcript)

1. **Pristine deployment**: `metaEvidenceUpdates()` → `0`; slots 0–16 recorded.
   Layout cross-checks asserted: slot 0 = arbitrator
   (`0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002`), slot 13 = `itemList` length
   (3). Slots 1 (arbitratorExtraData head), 3 (governor),
   8 (challengePeriodDuration = 1) and 10–12 (stake multipliers
   `0x2710/0x4e20/0x2710`) are visible in the transcript's BEFORE block,
   matching the seeder's deploy parameters.
2. **Governor `changeMetaEvidence` #1** — tx
   `0x361f94dc697b9bf1dcbdb7d3b241ee75ec99247bad558aca180e17731e88e7e3`,
   block 47881787, status `0x1`. Asserted: getter → **1**, slot 9 → `0x…01`,
   and **every other SAMPLED slot (the probe samples slots 0–16)
   byte-identical** to the BEFORE read. The sample covers the contract's
   scalar-field region around the counter; slots outside it (mapping/array
   payload slots) are not enumerated by this probe.
3. **Governor `changeMetaEvidence` #2** — tx
   `0x9c08bb7504cdd905819c6ad59295f87522673f1a65aa4fb868e28976c75f4241`,
   block 47881788, status `0x1`. Asserted: getter → **2**, slot 9 → `0x…02` —
   the observed transition is an increment, consistent with the source below.

## Source pin: the no-reset property

Behavioral probing shows which slot the counter lives in and that observed
transitions increment; it cannot show that NO reachable path resets the
counter. That property is pinned from the verified source of a mainnet
deployment of the SAME runtime generation:

- **Instance**: `0x54A92C21c6553a8085066311F2C8D9Db1B5e6610` — the Gate 1
  canary, returned by `GTCRFactory.instances(3)` on Gnosis, runtime codehash
  equal to the pinned constant (established in
  `docs/spikes/gnosis-anchor-results.md`).
- **Verification**: Gnosis Blockscout, contract `GeneralizedTCR`, compiler
  `v0.5.17+commit.d19bba13`, status *partially verified* (runtime bytecode
  match; the metadata hash is not additionally attested). Retrieval, exactly
  as the probe script re-executes and ASSERTS it on every run:
  `curl -sL https://gnosis.blockscout.com/api/v2/smart-contracts/0x54A92C21c6553a8085066311F2C8D9Db1B5e6610`,
  extracting the JSON `source_code` field. sha256 of the retrieved flattened
  source (digest-pinned in the script; the probe fails if a re-retrieval ever
  hashes differently):
  `2cf70f05773971382aa44e2ec5e9752f4e23a0edbaef1b1cb5230bc6c707e0d3`
  (49,115 bytes — labeled in BYTES; an earlier figure of 49,114 was a
  character count).
  The probe also asserts the API's verification fields (name
  `GeneralizedTCR`, the EXACT compiler string `v0.5.17+commit.d19bba13`,
  `is_verified` and
  `is_partially_verified` both true, `is_changed_bytecode` FALSE), the
  source's occurrence counts (6 total, exactly one `metaEvidenceUpdates++`
  write, zero `assembly`, `delegatecall`, and `selfdestruct` occurrences),
  AND — closing the source→canary→reviewed-bytecode chain inside the same
  reproduction — that the canary's on-fork runtime codehash
  (`cast keccak(cast code)`) equals the pinned constant the probed registry
  deployment also hashes to.
- **Every occurrence of `metaEvidenceUpdates` in that source** (6 total): the
  declaration (`uint public metaEvidenceUpdates;`), ONE write —

  ```solidity
  function changeMetaEvidence(string calldata _registrationMetaEvidence, string calldata _clearingMetaEvidence) external onlyGovernor {
      metaEvidenceUpdates++;
      emit MetaEvidence(2 * metaEvidenceUpdates, _registrationMetaEvidence);
      emit MetaEvidence(2 * metaEvidenceUpdates + 1, _clearingMetaEvidence);
  }
  ```

  — and four pure reads (the two event IDs above and the two
  `request.metaEvidenceID` bindings). The source contains **no `assembly`
  block, no `delegatecall`, and no `selfdestruct`**; the constructor never
  assigns the field (implicit zero).
- **Bytecode-level confirmation** (source text alone cannot exclude
  compiler-emitted delegation, e.g. through a linked public library): the
  probe disassembles the canary's exact on-fork RUNTIME. The trailing
  Solidity CBOR metadata is FULLY PARSED, not merely sniffed (round-7): the
  probe asserts the exact expected solc-0.5.17 structure — a two-entry map
  `{"bzzr1": bytes(32), "solc": bytes(3) == 00 05 11}`, total length exactly
  50, consumed exactly — so the executable boundary is validated, and a wrong
  future pin cannot silently strip trailing executable bytes. The remaining
  16,154-byte executable body is then walked with PUSH immediates skipped
  (a PUSH immediate running past the body end is itself an assertion
  failure), asserting that **no executable `DELEGATECALL` (0xf4), `CALLCODE`
  (0xf2), or `SELFDESTRUCT` (0xff) opcode exists**. (The metadata blob itself
  contains an 0xf2 byte, which is exactly why the parse precedes the scan.)
  Together with the source facts above, the deployed generation has **no
  feasible transition from nonzero back to zero**.

## Semantics

- The CONSUMER verifier's invariant (the `intend` CLI + spec §6 step 3b / §8
  point checks — companion changes, per the note at the top) is
  `metaEvidenceUpdates == 0` proven at every anchor. The Gate 2 spike's
  verifier here does not check slot 9 (it predates the owner decision).
- **The no-reset property is LOAD-BEARING for that invariant.** A `== 0` proof
  at one anchor establishes "never updated as of that anchor" only because no
  path returns the counter to zero: if the bytecode had a reset path, a
  `0 → 1 → 0` excursion BETWEEN two observed anchors would be invisible to
  both proofs, and a temporarily swapped policy could have judged requests
  unnoticed. The property is therefore pinned above (source + codehash), not
  assumed — and monotonicity is not merely why a once-updated registry stays
  non-compliant; it is part of why the zero proof means what the spec claims.
- One theoretical exception, stated for exactness: the counter is a uint256,
  so 2^256 governor increments would wrap it back to zero. That is
  infeasible, which is why the record's formulation throughout is "no
  FEASIBLE transition" rather than an absolute "never".
- As with slots 13/14, both the slot constant and the no-reset property are
  properties of the exact codehash, void against any other bytecode
  (spec §3/§5).
