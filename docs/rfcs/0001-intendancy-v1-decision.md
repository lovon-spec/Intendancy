# RFC 0001: Intendancy V1 Owner Decision

**Status:** Accepted architecture; implementation parameters remain provisional<br>
**Date:** 2026-08-23<br>
**Scope:** Intendancy's first skills registry and the `intend` package-manager verifier

This record is the canonical disposition of the RFC 0001 architecture brief and its reviews. The review documents remain useful design history, but any conclusion in them that conflicts with this record is superseded.

## 1. V1 product boundary

V1 proves the concrete Intendancy/Intendant package-manager path before generalizing the machinery:

- one unmodified Classic Kleros GeneralizedTCR, deployed through the official factory on Gnosis Chain;
- one skills-registry verifier profile with locally pinned chain, contract, bytecode, storage-layout, descriptor, and policy bindings;
- one binary providing catalog update, exact-item installation, and audit;
- internal seams for consensus, proof, CAR/UnixFS, registry, and package logic, without a public adapter or projection framework.

The broader `intend` verifiable-materializer idea remains a direction to extract from working implementations, not a framework V1 must build first.

## 2. Snapshot and trust anchor

An untrusted CDN, indexer, gateway, or mirror may provide the bulk snapshot and proof bundle. A conforming verifier authenticates:

1. the Classic registry account and pinned runtime code;
2. `itemCount` and every append-only `itemList` position;
3. each descriptor by re-hashing it to its proven item ID; and
4. every item's status at the same anchor through storage proofs.

The resulting claim is **complete enumeration with statuses as of the anchor**. It is not a proof of historical status, past content availability, or state after that anchor. Because statuses can change without `itemCount` changing, every catalog refresh must authenticate a fresh status sweep unless a future registry supplies an authenticated transition mechanism.

The target strict mode embeds Gnosis consensus verification and derives a finalized execution `stateRoot` locally. Until that path is production-ready, two explicitly degraded modes may be offered:

- **Header quorum:** accept a finalized-header claim from independently operated providers, then verify account and storage proofs locally against its `stateRoot`.
- **RPC quorum:** use agreeing RPC results as the compatibility floor.

Only embedded consensus supports the unqualified trustless claim. All modes must report their anchor source, finality/freshness result, and trust grade rather than silently falling back.

Clients persist a per-registry finalized-anchor high-water mark. A provider cannot roll update, install, or audit back to an older block—or substitute a different hash at the same height—without an explicit operator recovery action.

## 3. Package-manager semantics

```text
intend update
  -> authenticate a complete Classic snapshot
  -> report anchor mode, enumeration scope, and freshness

intend install <exact-item-id>
  -> require a fresh finalized status of exactly Registered
  -> fetch the skill tree from any provider
  -> verify the complete CAR/UnixFS DAG against the Tree CID
  -> reject unsafe paths and symlink escapes
  -> install that exact tree and write an exact lockfile

intend audit
  -> verify installed bytes against the lockfile
  -> refresh the item's finalized status
  -> apply the suspension or disable rule below
```

The lockfile records at least the chain, registry, item ID, Tree CID, and verified finalized anchor. It identifies exact content, not a name, range, mutable channel, or inferred latest version.

Status handling is requester-neutral:

- `Registered`: eligible for installation or loading after all local checks pass.
- `RegistrationRequested`: not installable.
- `ClearingRequested`: retain the bytes and lockfile for inspection, but suspend automatic loading and execution by default. A deliberate local override may be offered.
- `Absent` after a locally recorded registered state: hard-disable, but retain forensic material rather than silently deleting it.
- A later return to `Registered` permits re-enabling only after a fresh finalized check and according to local policy.

## 4. Review-period availability

V1 follows ordinary pragmatic Curate practice. Throughout a registration request's challenge period, the complete skill tree identified by the Tree CID must be stored on IPFS and remain accessible and discoverable. Failure is a registration-policy violation.

Curate remains optimistic: an unchallenged request may list without juror review. If challenged, jurors later assess evidence about whether the submission complied during the challenge period; a juror-time fetch neither proves continuous earlier availability nor, by itself, disproves an evidenced earlier outage. V1 does not invent an automatic cure rule or claim that historical uptime is cryptographically provable.

Submission tooling should upload and pin through the established Kleros path by default and may add mirrors. This is operational reliability, not a new trust or proof layer. V1 has no ingestion certificates, adjudicative or informational receipts, recognized-archiver roster, possession labels, or availability oracle.

## 5. Descriptor and policy decisions

The descriptor retains exactly six ordered UTF-8 string columns:

1. Name
2. Description
3. Tree CID
4. Runtimes
5. Origin
6. Reserved

Column 6 is present and **must be the empty string** under this policy version. It has no bond, predecessor, lineage, or extension semantics. Assigning it meaning requires a later RFC and versioned policy change.

If Origin is present, the provenance evidence must bind the exact submitted semantic skill tree. A publisher may attest the literal Tree CID; a pinned git commit may establish the same relative paths, node kinds, file bytes and executable bits, and symlink targets under the equivalence rules in the listing policy. Mere repository, commit, domain, or publisher association is insufficient.

> **Amendment (owner decision, 2026-08-24).** The semantic-tree sentence above is
> superseded for V1: skill trees consist of directories and regular files ONLY —
> symlinks are prohibited, and executable bits are **not semantic** (installers
> neither preserve nor interpret them; scripts run via explicit interpreters).
> The git-equivalence rules therefore compare relative paths, node kinds
> (directory/regular file), and file bytes; an exec-bit-only difference still
> binds, and a commit containing symlinks binds no listable tree. Normative text:
> listing-policy v2.1 (criteria 2 and 6). A second owner decision of the same
> date pins ONE IMMUTABLE POLICY PER REGISTRY: consumers prove the registry's
> `metaEvidenceUpdates` counter is zero at every verification anchor, and a
> policy change requires a new registry deployment (verified-snapshot-spec §6
> step 3b).

There is no guardian roster or privileged requester identity. Any party may use the stock removal-request path, and clients apply the same `ClearingRequested` suspension regardless of who paid the request deposit.

## 6. Explicit V1 exclusions

V1 does not include:

- automatic upgrade, release lineage, predecessor resolution, canonical-head selection, or mutable package channels;
- generalized registry adapters, adapter plugins, generic projection manifests, or a public capability-taxonomy API;
- T2CR, LGTCR, Token Lists, PGTCR/Stake Curate, or sidecar catalog commitments;
- receipts, ingestion certificates, archiver or guardian rosters, or stronger review-period DA machinery;
- stateless EVM execution or proof-verified arbitrary view calls.

Moving from one listed release to another is an explicit user selection followed by the same exact-item install checks.

## 7. Gates and provisional parameters

Two bounded validations precede a wire-format or implementation freeze:

1. Demonstrate `Gnosis checkpoint -> consensus finality -> execution stateRoot -> known EIP-1186 account/storage proof`, including stale-checkpoint, fork-boundary, and adversarial cases.
2. Benchmark provider generation and consumer verification for complete 1,000- and 10,000-item Classic snapshots, including proof deduplication, transfer size, CPU, memory, resource limits, CAR verification, path sanitization, and exact lockfile installation.

This decision does not freeze the registry address, runtime code hash, arbitrator proxy/implementation pins, storage slots, encoder vectors, Agent Skills specification commit, snapshot framing, checkpoint sources, maximum anchor age, size/resource limits, deposits, court parameters, governor/governance arrangement, or other production constants. Those require measured validation and explicit pre-deployment sign-off. No production deployment or funds are authorized by this architecture decision.

## 8. Amendment: the registry is unbranded (owner decision, 2026-09-04)

The registry is deployed as neutral infrastructure named **Agent Skills Registry**. Its MetaEvidence, listing policy, logo and juror evidence display carry no product branding; Intendancy names the consumer product (the `intend` distribution, the website, the profile). The policy discloses the governor and its limits, and describes the wider-governance path as the registry's community without naming an organization. Nothing in the contracts or the deployment changes; names live in MetaEvidence and documents, which are immutable per registry, which is why this was decided before anything was pinned.

## 9. Historical record

The following documents are retained as the review trail:

- [Architecture review brief](./0001-intend-architecture-review.md)
- [Claude review](./0001-review-claude.md)
- [Codex counter-review](./0001-review-codex.md)
- [Claude convergence response](./0001-review-claude-2.md)
