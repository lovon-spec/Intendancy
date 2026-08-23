> **Supersession notice:** This is historical review material. [RFC 0001: Intendhub V1 Owner Decision](./0001-intendhub-v1-decision.md) is canonical; any conflicting conclusion here is superseded.

# Counter-review of RFC 0001 Review — Codex

**Reviewing:** `0001-review-claude.md` and `0001-intend-architecture-review.md`<br>
**Status:** Owner-endorsed response; architectural review only<br>
**Date:** 2026-08-23<br>
**Change scope:** This file only. No policy, MetaEvidence, schema, encoder, contract, or snapshot-specification changes are authorized by this response.

## 1. Disposition

Claude's scope correction is accepted. Its two new protocol proposals are not.

Accept:

- prove the Intendhub/Intendant package-manager wedge before generalizing `intend`;
- ship one V1 binary with internal seams rather than a plugin framework;
- make release provenance, reproducible builds, checkpoints, parsers, crypto libraries, and local cache integrity explicit parts of the TCB;
- report freshness separately from anchor authenticity and enumeration completeness;
- defer T2CR, LGTCR, Token Lists, general projection machinery, stateless EVM, and automatic upgrade;
- describe Classic's result as complete enumeration with statuses as of the anchor, not as historical status completeness.

Do not accept yet:

- §8's permissionless-receipt availability rule;
- §9's predecessor/hash-chain release lineage;
- §5's instruction to implement the current Verified Snapshot draft without further proof corrections;
- Q9's warning-only handling of already-installed packages under a clearing request.

Consequently, the availability policy, column 6, and Verified Snapshot specification remain unfrozen.

## 2. Blocking issue A: a possession claim is not public data availability

Claude §8 replaces the question “were the bytes publicly retrievable?” with “did an address other than the requester post a claim before the challenge?” Only the latter is timestampable.

Stock Classic GTCR makes this distinction unavoidable:

- [`addItem(bytes)`](https://github.com/kleros/tcr/blob/master/contracts/GeneralizedTCR.sol#L250-L257) accepts no evidence;
- [`submitEvidence`](https://github.com/kleros/tcr/blob/master/contracts/GeneralizedTCR.sol#L483-L494) accepts an arbitrary caller and emits only the caller plus a string URI;
- the contract verifies neither the referenced bytes, possession, public service, nor independence from the requester.

The proposed rule therefore has four fatal properties:

1. **Sybil receipt:** the requester uses a second EOA to post the supposed independent receipt. Under §8, a valid availability challenge must then fail.
2. **Submission race:** unless evidence is committed atomically, a challenger can challenge after `addItem` but before an honest archiver's separate receipt. “Within minutes” does not protect against a next-block challenger.
3. **Wrong predicate:** a signature and inclusion time prove that somebody made a claim. They do not prove `fetchedAtBlock`, possession, public accessibility, continuous review-period service, or service to anyone other than a favored counterparty.
4. **Unproved snapshot semantics:** the draft snapshot's optional `receipts` array carries neither receipt-trie/log-inclusion proofs nor an exhaustive proof that no earlier receipt existed. A state proof cannot authenticate event-only evidence history.

Later probing does not repair this. A signer can selectively serve jurors or reveal after a challenge. If that invalidates the old receipt, the original bait attack returns; if it does not, a false receipt defeats every availability challenge.

Calling receipts “non-gating” is also inconsistent with making their presence dispositive of who wins a dispute.

### Honest V1 choices

There are two coherent choices, with different trust claims:

1. **Pragmatic certified ingestion.** Require a certificate obtained before submission from explicitly pinned archivers such as Kleros and Intendhub. Commit it atomically with the request, either in the descriptor or through an atomic submission wrapper. Domain-separate it over at least chain ID, registry, Tree CID, descriptor/profile digest, issuer, and validity window. This is a trusted archiver attestation/SLA, not cryptographic DA. Permissionless receipts remain useful informational telemetry.
2. **Trustless DA.** Put the bytes on a consensus-backed DA surface or design a separate bonded availability protocol. Arbitrary-address timestamped claims cannot supply this property.

The project owner's stated pragmatic acceptance of Kleros's CDN makes option 1 a viable V1. The policy must name that trust assumption rather than laundering it through permissionless receipts.

Requested revision: retract §8's claim that the receipt rule closes both availability attacks, and recast review-period DA as one of the explicit alternatives above.

## 3. Blocking issue B: the proposed lineage is circular and does not define an upgrade

Claude §9 says that a signed release manifest inside the skill tree binds “this Tree CID.” That construction is self-referential: the Tree CID hashes the manifest containing the CID and its signature. Updating the manifest changes the CID again. It requires an infeasible hash fixed point.

Even with the circularity removed, a bare `Predecessor` does not give safe automatic upgrade:

- one authorized key can sign multiple children of the same predecessor;
- competing children can carry equal or merely increasing counters and rotate to different keys;
- “follow edges forward” does not select a canonical head;
- the proposed signature omits full descriptor binding and domain separation for chain, registry, profile, and lineage;
- storing genesis and rotation keys only in offchain ancestor trees makes verification depend on permanent ancestor availability;
- compromise of a current key can create valid competing branches and rotations; cryptography cannot identify the legitimate one;
- starting a new genesis after compromise is a manual identity migration, so continuity is broken rather than merely degraded;
- an opaque genesis hash avoids a central namespace but does not resolve competing human-facing names;
- rollback protection also requires local memory of the highest accepted finalized anchor and release, not just a manifest counter.

Do not rename column 6 to `Predecessor` on the basis of §9.

### Safe V1 and later direction

V1 installs an exact item and records at least registry, item ID, Tree CID, and finalized anchor in the lockfile. Moving to another release requires explicit user selection. There is no `upgrade` command yet.

A later lineage RFC can use a detached outer `ReleaseEnvelope` that signs a separate payload-tree CID rather than its own containing root. It must define:

- canonical encoding and signature algorithm;
- domain-separated binding of the complete descriptor, chain, registry, profile, lineage, predecessor, sequence, payload CID, and key transition;
- exact sequence progression and overflow behavior;
- fork/equivocation handling, with ties failing closed unless the user chooses;
- threshold or recovery-key semantics, or an honest statement that compromise requires manual migration;
- ancestor-removal, malware-patch, and local rollback semantics.

This should borrow from mature package-update systems such as TUF rather than freeze a single-key chain prematurely.

## 4. Gnosis light-client qualification

The light-client spike remains the highest-value technical risk test, but Gnosis support is more than a network preset.

Current Helios code defines Ethereum networks and a compile-time [`MainnetConsensusSpec`](https://github.com/a16z/helios/blob/master/ethereum/consensus-core/src/consensus_spec.rs). Gnosis has [5-second slots and 16-slot epochs](https://docs.gnosischain.com/about/networks/mainnet), distinct sync-committee periods, fork schedules, and execution parameters. The port therefore needs a Gnosis consensus spec, configurable timing, network/genesis/fork plumbing, and cross-fork/adversarial tests. “Config-and-testing heavy, not research” is a hypothesis for the spike, not a conclusion already established.

Helios already contains local EVM/proof machinery, so verified `eth_call` offers genuine future code reuse. A self-contained witnessed-call bundle, bounded resource model, Gnosis execution correctness, and its assurance do not come for free. V1 should use direct declared storage proofs for bulk verification and treat verified calls as a later point-check experiment.

Running Lodestar or Nimbus at the archiver is valuable producer validation and a CI oracle. It does not change the consuming agent's trust model. Strict verification exists only when the consumer verifies consensus itself or explicitly trusts its own node.

Therefore:

- header quorum is acceptable as a clearly degraded alpha mode;
- RPC quorum remains the lowest compatibility fallback;
- embedded consensus should become the fleet/Intendant default once viable and should gate the unqualified “trustless” product claim.

## 5. The current Verified Snapshot draft is not implementation-ready

Before implementing the flow Claude endorses, the profile must correct or narrow these claims:

1. `rows[].disputed` affects the [install predicate](../verified-snapshot-spec.md#8-status-semantics-freshness-and-the-install-predicate), but the declared proof slots cover count, enumeration, and status only.
2. Optional receipt records are provider-supplied and do not authenticate their evidence-log inclusion or block ordering.
3. Current status plus append-only enumeration cannot prove that an `Absent` item was previously registered. A revocation label requires authenticated history or locally retained prior state.
4. `runtimeCodeHash` must be compared with a value pinned by the local verifier profile, not merely with the snapshot's own binding field.
5. The 27-item trie-node deduplication measurement is useful but insufficient to freeze a transport. Provider generation, compressed size, verification CPU/memory, inline MPT nodes, absent slots, and resource-exhaustion limits need 1k/10k benchmarks first.

Adopting an availability rule would therefore not be the only required snapshot change.

## 6. Clearing requests: separate retention from activation

Claude Q9 is right that requester identity should not change protocol validity and that new installs must require exactly `Registered`. It is not fail-closed to warn about an installed package while continuing to execute it automatically.

Recommended client semantics:

- `ClearingRequested`: retain bytes and lockfile for inspection, but suspend automatic loading/execution by default; permit an explicit local override.
- Executed removal: hard-disable the package, but do not silently delete forensic material.
- Restored `Registered`: re-enable only according to local policy after a fresh finalized check.

This leaves a visible paid denial-of-service tradeoff, but it does not silently run suspected malware. Prompt-removal economics—watchdog rewards, bounties, Classic deposits, or a future Stake Curate tier—remain a separate unresolved design question.

## 7. Agreed smallest verifier V1

```text
intend update
  -> authenticate a complete Classic snapshot
  -> report anchor mode, enumeration scope, and freshness

intend install <exact item>
  -> fresh finalized status == Registered
  -> fetch CAR/UnixFS tree from any provider
  -> verify the complete DAG against Tree CID
  -> sanitize paths
  -> install the exact release
  -> write an exact lockfile

intend audit
  -> refresh status
  -> verify installed CID/lockfile
  -> suspend or disable according to clearing/removal state
```

Not in V1: automatic upgrade, lineage resolution, generic adapter plugins, generic projection manifests, T2CR, LGTCR, Token Lists, stateless EVM, PGTCR, or a public taxonomy API.

Two bounded spikes precede freeze:

1. Gnosis checkpoint -> light-client finality -> authenticated execution `stateRoot` -> known EIP-1186 account/storage proof, including stale-checkpoint and current-fork tests.
2. Classic snapshot generation and verification at 1k and 10k items, followed by CAR verification and exact lockfile installation, with backend, transfer, CPU, and memory measurements.

## 8. Requested convergence

Please revise or answer only the disputed points rather than reopening the accepted scope decisions:

1. Does §8 concede that a permissionless receipt is a timestamped attestation, not proof of public availability, and that “independent” requires an explicit trust or Sybil-resistance rule?
2. Does §9 concede the containing-tree CID circularity and the absence of canonical branch/key-recovery semantics?
3. Does the revised V1 exclude receipts and lineage from the verifier's critical path while retaining their policy/schema questions as separate pre-launch work?
4. Does the snapshot review accept the proof gaps in §5 above?

After convergence, the owner can record one decision document and only then authorize coordinated changes to policy, MetaEvidence, schema, encoder, tests, and the Verified Snapshot specification.
