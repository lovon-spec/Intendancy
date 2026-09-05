# Evidence record: `arbitrator` and `arbitratorExtraData` storage slots

**Status: PROVISIONAL pending reviewer acceptance** (spec §5, the same footing as the `metaEvidenceUpdates` slot). Pinned for runtime codehash `0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d` only; void against any other bytecode.

## Claims

1. `arbitrator` (the `IArbitrator` every dispute is created on) is storage slot **0**, an address in the low 20 bytes with the upper 12 bytes zero.
2. `arbitratorExtraData` is storage slot **1**, a Solidity `bytes`: short form (length < 32) inline with `length × 2` in the low byte; long form (length ≥ 32) with `length × 2 + 1` in the word and the data at `keccak256(uint256(1)) + i`.
3. A governor `changeArbitrator(address, bytes)` moves exactly these words, in both forms, and no other sampled slot (2..16).

## Observation, 2026-09-04, anvil fork of Gnosis (registry deployed through the real `GTCRFactory` by the Gate 2 spike's `seed`)

Registry `0x2a25ec70329918d13ec23477716a159d063735d0`, codehash equal to the pin.

| read | value |
|---|---|
| `arbitrator()` | `0x9C1dA9A04925bDfDedf0f6421bC7EEa8305F9002` |
| slot 0 | `0x…9c1da9a04925bdfdedf0f6421bc7eea8305f9002` |
| `arbitratorExtraData()` | 64 bytes: court `0`, jurors `3` |
| slot 1 | `0x…81` = 64 × 2 + 1 |
| `keccak256(1) + 0` | `0x…00` (court) |
| `keccak256(1) + 1` | `0x…03` (jurors) |
| slot 3 | the governor (cross-check with the slot-9 record) |

After `changeArbitrator(0x…dEaD, 0xaabbcc)` from the governor: slot 0 `0x…dead`; slot 1 `0xaabbcc00…06` (short form, low byte 3 × 2). After `changeArbitrator(0x9C1d…9002, court 19, jurors 3)`: slot 0 restored; slot 1 `0x…81`; `keccak256(1) + 0` = `0x…13`; `keccak256(1) + 1` = `0x…03`.

## Reproduction

`cli/tools/probe-arbitrator-slots.sh` performs the observation above as assertions (A. codehash; B/C. both slots against the getters, long form; D. the short form and the unchanged sampled slots under a governor change; E. the long form restored) on a fresh fork and registry, writing a digest-recorded transcript. Run end to end on 2026-09-05 by the author on a fresh fork and registry: `ALL ASSERTIONS PASSED`, transcript sha256 `ed92eca8beb58bb247bd1c497655c7795b14002cac424cc2ff99a667dc3d8334`. The values above were first read by hand the same way on 2026-09-04. Not yet reproduced by a reviewer, which is what would lift the PROVISIONAL status.

The `intend` CLI's own end-to-end smoke (`cli/tools/smoke.sh`) on a fork with 26 items reports `slotProofsChecked: 58`, that is `2 × 26 + 2` plus the arbitrator slot and the three words of the 64-byte extra data, so the generation and verification paths both cover the new slots.

## Why the pin exists

The arbitrator is who decides every later verdict, and the extra data is which court and how many jurors. A governor may switch them, and to a consumer that is a policy-grade change: silently following it would let a compromised or captured governor move the registry to a court of its choosing without anyone noticing. The `intend` verifier therefore proves both at every full verification and every fresh point check (spec §6 step 3c, §8) and fails closed on a mismatch; a switch is accepted only through a new signed profile release. The planned adoption of Intendment's arbitrator-level wrapper is exactly such a release.
