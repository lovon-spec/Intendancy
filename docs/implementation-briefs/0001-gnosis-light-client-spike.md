# Implementation Brief 0001: Gnosis Finalized-State Anchor Spike

**Status:** Ready for implementation<br>
**Implementer:** Claude<br>
**Reviewer:** Codex<br>
**Scope owner:** `spikes/gnosis-anchor/**` and `docs/spikes/gnosis-anchor-results.md` only

## Why this is next

Intendancy V1 depends on one claim that is still unproven in this repository: a small consumer can start from an explicit Gnosis weak-subjectivity checkpoint, verify consensus light-client updates, obtain a finalized execution `stateRoot`, and use that root to verify the Classic GTCR account and storage proofs supplied by an untrusted provider.

This is the highest-risk architecture assumption in [RFC 0001](../rfcs/0001-intendancy-v1-decision.md). Test it before implementing the package manager or freezing the [snapshot wire format](../verified-snapshot-spec.md).

Gnosis currently exposes a consensus RPC and documents 5-second slots, 16-slot epochs, and checkpoint-sync infrastructure. Its consensus specification and fork schedule differ from Ethereum mainnet. Helios is a plausible Rust starting point, but its current Gnosis compatibility must be demonstrated rather than assumed. Initial source inspection indicates that current Helios still selects an Ethereum `MainnetConsensusSpec` internally; a custom TOML file alone does not change SSZ bounds. Verify this against a pinned revision, then make the smallest isolated, upstreamable Gnosis preset patch or document a better maintained primitive.

Primary references:

- [Intendancy V1 decision](../rfcs/0001-intendancy-v1-decision.md)
- [Draft verified-snapshot specification](../verified-snapshot-spec.md)
- [Gnosis mainnet parameters and endpoints](https://docs.gnosischain.com/about/networks/mainnet)
- [Gnosis consensus/execution specifications](https://github.com/gnosischain/specs)
- [Gnosis canonical network configuration](https://github.com/gnosischain/configs)
- [Helios](https://github.com/a16z/helios)
- [EIP-1186 `eth_getProof`](https://eips.ethereum.org/EIPS/eip-1186)

## Goal

Produce a runnable, non-production Rust spike which performs this exact chain:

```text
explicit trusted Gnosis checkpoint
  -> verify bootstrap and sync-committee updates from an untrusted Beacon API
  -> obtain a finalized Gnosis execution payload header
  -> expose its block number, block hash, timestamp, and stateRoot
  -> fetch an EIP-1186 proof at that exact execution block
  -> verify the account proof against stateRoot
  -> require the locally pinned contract runtime codehash
  -> verify at least one Classic GTCR storage value against the proven storageRoot
```

The successful output is evidence for the strict-proof architecture. It is not a production light client, package manager, snapshot server, or generic verified-RPC framework.

## Working-set rules

Claude owns only:

- `spikes/gnosis-anchor/**`
- `docs/spikes/gnosis-anchor-results.md`

The checkout already contains uncommitted V1 architecture, policy, frontend, MetaEvidence, and deployment changes. Do not revert, reformat, stage, or edit them. Do not change the canonical RFC, listing policy, snapshot spec, contracts, or frontend during this spike. If the spike disproves an assumption, record the finding in the results document for Codex review; do not silently rewrite the architecture.

Use exact released crate versions or a pinned upstream git revision and commit the lockfile. Pin the exact Gnosis config/spec revisions used as well; never copy parameters silently from a mutable default branch. Do not vendor an upstream repository. If Helios needs Gnosis support, keep the smallest adapter or patch isolated inside the spike and record the upstream delta precisely.

## Required interface

The executable may choose its internal architecture, but its live mode must accept equivalent explicit inputs:

```text
--checkpoint <0x beacon block root>
--consensus-rpc <Beacon API URL>
--execution-rpc <Execution JSON-RPC URL>
--fixture-profile <known Classic GTCR profile>
--max-checkpoint-age <duration>
```

Security requirements:

1. `--checkpoint` is mandatory. There is no automatic checkpoint download, community fallback, or silent default.
2. The consensus and execution endpoints are untrusted data sources. Agreement with an RPC-reported `finalized` tag is not verification.
3. The finalized execution header, including `stateRoot` and `blockHash`, must be authenticated through the verified light-client update and execution branch.
4. Fetch `eth_getProof` for the exact finalized execution block. Proof validity comes only from local Merkle-Patricia verification against that authenticated `stateRoot`.
5. The target address, expected runtime codehash, storage-layout constants, and requested slots come from a local fixture profile—not from provider output.
6. Reject stale checkpoints according to the explicit maximum age. Persist or model the per-registry high-water rule: older block numbers and conflicting hashes at the same height fail unless an explicit recovery path is selected.
7. Do not reduce the result to one `verified: true` flag. Report checkpoint, consensus finality, execution anchor, account proof, codehash, and storage proof as separate stages.

A successful machine-readable result should contain at least:

```json
{
  "mode": "strict-proof-spike",
  "chainId": 100,
  "checkpoint": "0x...",
  "finalizedBeacon": { "slot": 0, "blockRoot": "0x..." },
  "execution": {
    "blockNumber": 0,
    "blockHash": "0x...",
    "stateRoot": "0x...",
    "timestamp": 0
  },
  "account": {
    "address": "0x...",
    "codeHash": "0x...",
    "storageRoot": "0x..."
  },
  "storage": [{ "slot": "0x...", "value": "0x..." }]
}
```

## Fixture choice

Use a real, factory-deployed **Classic** GeneralizedTCR on Gnosis with at least one item. The results document must identify:

- registry address;
- official factory `NewGTCR` deployment transaction/log;
- runtime codehash;
- anchored execution block number/hash/stateRoot;
- the exact proven slot and its decoded meaning;
- why the chosen bytecode and layout are valid for the fixture.

At minimum, prove `itemList.length` and one `itemList[i]` or item-status slot. Recompute the slot locally. A provider-supplied slot key is not authoritative. An independent `eth_call` comparison may be printed as a diagnostic but must not participate in proof acceptance.

Use this known canary unless independent verification finds it unsuitable:

| Field | Seed value to verify and pin |
|---|---|
| Factory instance | Classic GTCR instance 3 |
| Registry | `0x54A92C21c6553a8085066311F2C8D9Db1B5e6610` |
| Runtime codehash | `0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d` |
| Slot | `0x0d` (`itemList.length`) |
| Expected value at seed anchor | `0x1b` (27) |
| Finalized beacon slot | `29703968` |
| Finalized beacon root | `0xfc932dad7edb0aee6f2e73d5943ab491dad08eec0470c5971ea31329dcfc72f0` |
| Execution block | `47878094` |
| Execution block hash | `0x4f14c103edae0b715a2f2fd48cbaf473d01087b6eb47013a379c1e9a4337deb7` |
| Execution state root | `0xd4112ea2939c738fa50bd0e1653962111c5db2f9564b1d298a65964fef704298` |

These are fixture seeds, not trusted constants merely because this brief lists them. Regenerate and verify their provenance. Most importantly, the trusted checkpoint for the successful test must be **earlier** than the finalized target: using the target beacon root itself as the checkpoint would demonstrate bootstrap acceptance, not signed advancement and finality.

## Deliverables

### 1. Isolated Rust crate

Create `spikes/gnosis-anchor/` containing:

- `Cargo.toml` and committed `Cargo.lock`;
- a small CLI or example binary;
- separated consensus-anchor and EIP-1186 proof-verification modules;
- local Gnosis network/fork configuration;
- bounded parsers and explicit error types;
- a README with exact offline and live commands.

Prefer maintained cryptographic, SSZ, BLS, RLP, and trie-proof implementations. Do not write bespoke BLS verification or a novel Merkle-Patricia verifier unless no suitable reviewed implementation exists; justify any exception in the results document.

The Gnosis preset must cover its non-mainnet values, including 16 slots per epoch, its sync-committee period/size, withdrawal bounds, genesis/fork versions, and the schedule through the currently active Fulu schema. Derive them from the pinned canonical Gnosis sources rather than from this brief.

### 2. Deterministic offline fixtures

Check in the smallest practical fixture set needed to rerun verification without trusting live endpoints. Deduplicate or compress large responses when appropriate, but keep provenance and a regeneration command. Fixtures must not contain API keys.

The default test suite must be offline and deterministic. Live network tests must be separately selected and clearly labeled.

### 3. Adversarial tests

Tests must demonstrate rejection of at least:

- wrong checkpoint/bootstrap root;
- invalid or insufficient sync-committee participation/signature;
- corrupted finality branch;
- corrupted execution-payload branch or substituted execution `stateRoot`;
- execution block-number/hash mismatch;
- malformed account proof;
- wrong locally pinned runtime codehash;
- malformed storage proof or substituted slot value;
- stale checkpoint;
- rollback to an older finalized anchor;
- conflicting block hash at the same height;
- use of Ethereum-mainnet constants or the wrong Gnosis fork schema.

Exercise the currently active Gnosis fork and at least one fork-boundary or sync-committee-period-boundary decoding case using an official vector or a recorded fixture. As of this brief, the public Gnosis light-client endpoint reports the post-Fusaka/Fulu schema; do not hard-code a pre-fork payload layout.

### 4. Results document

Write `docs/spikes/gnosis-anchor-results.md` with:

- dependency/revision choice and why;
- exact Gnosis parameters and fork schedule sources;
- commands and environment needed to reproduce;
- one successful live transcript summarized without secrets;
- offline test results;
- cold-start time, bytes downloaded, peak memory if readily measurable, and binary size;
- every remaining trust assumption;
- any upstream Helios/library patch required;
- a clear conclusion: `GO`, `GO WITH UPSTREAM PATCH`, or `NO-GO` for the strict anchor.

## Acceptance gate

The spike is ready for Codex review only when:

1. A fresh live run reaches a finalized Gnosis execution state root from an explicit checkpoint and verifies the Classic storage proof end to end.
2. The same verification passes offline from checked-in fixtures.
3. Every adversarial case above fails closed in a focused test.
4. No RPC response, downloaded checkpoint, provider-supplied codehash, or provider-supplied layout constant silently enters the trust base.
5. The results document makes unsupported, degraded, or untested behavior explicit.

A precise `NO-GO` is a valid spike result only if the implementation identifies the exact missing primitive or incompatible upstream assumption, includes the smallest reproducer, and sketches the bounded upstream change. “The library did not work” is not sufficient.

Operational warning: proof retention differs among free execution RPCs. During preparation, `rpc.gnosischain.com` could not serve `eth_getProof` as far back as the consensus-finalized execution block, while other public providers could. Keep the Beacon API and execution-proof RPC independently configurable; never replace the light-client anchor with the execution provider's `finalized` tag. Offline fixtures must make the default tests independent of provider retention.

## Explicit non-goals

Do not implement:

- the 1,000/10,000-item snapshot benchmark;
- the final snapshot JSON/binary format;
- CAR/UnixFS installation;
- the `intend update/install/audit` product CLI;
- checkpoint distribution or release signing policy;
- header-quorum or direct-RPC fallback modes;
- a general reverse-oracle adapter API;
- production deployment or contract changes.

## Handoff to Codex

When finished, leave the worktree intact and provide:

1. the complete changed-file list;
2. the dependency and trust-boundary summary;
3. exact offline test, live test, format, lint, and build commands;
4. the live fixture block/checkpoint identifiers;
5. known limitations and any test not run;
6. the results-document conclusion.

Codex will review cryptographic trust boundaries, Gnosis fork handling, fixture provenance, failure behavior, resource bounds, and whether the evidence actually satisfies the acceptance gate. Do not merge or generalize the spike before that review.
