# Intendancy V1 design

**Status:** Accepted architecture; proof transport and production parameters remain provisional.
**Canonical decision:** [`rfcs/0001-intendancy-v1-decision.md`](rfcs/0001-intendancy-v1-decision.md)

## Purpose

Intendancy gives Intendant and other agent runtimes a package catalog whose membership and artifact integrity can be checked without trusting a marketplace backend, CDN, IPFS gateway, indexer, or execution RPC.

V1 curates one artifact type: directories containing a standards-compliant `SKILL.md`. Plugins, MCP servers, and project conventions are intentionally deferred to separate registries and policies because their review criteria and risk differ.

## Product boundary

The conceptual layers are:

- **`intend`** — verification and package-management machinery: chain anchoring, state proofs, CID/CAR verification, safe installation, lockfiles, and auditing.
- **Intendancy** — the first registry profile: skills curated by an unmodified Classic Kleros GeneralizedTCR on Gnosis Chain.
- **Intendant** — the first consuming runtime.

V1 may ship these as one binary with internal seams. A generic adapter/plugin framework will be extracted only after a second real adapter demonstrates the common abstraction.

## Registry choice

Intendancy uses the unmodified Classic `GeneralizedTCR` deployed through Kleros's official Gnosis `GTCRFactory`.

Classic is useful here because it provides:

- an append-only `itemList` enumerable directly from authenticated contract storage;
- the exact descriptor bytes retrievable by item ID;
- `itemID = keccak256(descriptor)`;
- current status in a deterministic storage layout; and
- compatibility with Kleros factory discovery, Curate tooling, evidence, appeals, and deposits.

The project does not fork the fund-holding arbitration contract and does not add a catalog sidecar. A future native current-set root or history head remains an upstream improvement, principally for Light Curate and efficient deltas.

## Descriptor

An item is the RLP encoding of six UTF-8 strings:

| # | Field | V1 meaning |
|---|---|---|
| 1 | Name | Byte-identical to `SKILL.md` frontmatter `name`. |
| 2 | Description | Byte-identical to `SKILL.md` frontmatter `description`. |
| 3 | Tree CID | Canonical lowercase CIDv1, DAG-PB with a 32-byte SHA-256 multihash, for the complete UnixFS skill directory. |
| 4 | Runtimes | Declared compatible runtimes. |
| 5 | Origin | Optional provenance claim that must bind the exact submitted semantic skill tree, by literal-CID attestation or the policy's git-tree equivalence. |
| 6 | Reserved | Required to be empty. Any later meaning requires a versioned RFC and policy revision. |

The Tree CID binds every file and path in the skill. Mutable locators such as branches, tags, IPNS, DNSLink, or gateway URLs are not descriptor identities.

## Optimistic curation and availability

A registration request opens a challenge period. If nobody challenges it, it can become `Registered` without juror review. If challenged, jurors decide the dispute from the policy and submitted evidence.

The V1 availability rule follows established Curate policy:

> Throughout the registration request's challenge period, the complete skill tree identified by the Tree CID must be stored on IPFS and remain accessible and discoverable.

This is a temporal policy obligation, not a new data-availability protocol. Intendancy does not add ingestion receipts, certified archivers, issuer or guardian rosters, uptime probes, a late-production cure rule, or a juror-time retrieval test. Tooling should upload and pin through reliable infrastructure, including Kleros's service, but no particular gateway is normative.

Historical continuous availability remains evidentiary rather than cryptographically proven. After registration, unavailability alone is not a removal ground; unfetchable content simply cannot be installed.

## Verified retrieval

```text
untrusted CDN / indexer / mirror
  -> snapshot rows + deduplicated EIP-1186/MPT proof nodes
  -> finalized execution stateRoot authenticated by the client
  -> pinned Classic runtime codehash and storage layout
  -> complete itemList and status-at-anchor verification
  -> fresh finalized status check for the selected item
  -> CAR/UnixFS retrieval from any provider
  -> complete DAG and path verification against Tree CID
  -> sanitized exact install and lockfile
```

Three independent dimensions are always reported:

1. **Anchor strength:** embedded consensus, header quorum, or RPC quorum.
2. **Enumeration scope:** complete Classic enumeration at the anchor, or a weaker explicitly labeled result for future adapters.
3. **Freshness:** finalized block age and rollback/monotonicity state.

An embedded Gnosis proof-of-stake light client is required for an unqualified trustless claim. Header quorum is an explicitly degraded alpha mode. An RPC quorum is a compatibility floor, not a consensus proof.

The client persists a finalized-anchor high-water mark per registry and rejects older anchors or a conflicting hash at the same height unless the operator deliberately enters a recovery flow. A merely recent header is not sufficient rollback protection.

## Package-manager behavior

- `intend update` authenticates a complete catalog snapshot and reports its anchor, scope, and freshness.
- `intend install <exact-item>` requires a fresh finalized status of exactly `Registered`, verifies the full Tree CID DAG, sanitizes paths, installs exact bytes, and writes a lockfile.
- `intend audit` compares installed lockfiles with fresh status. A transition from a locally proven `Registered` state to `Absent` is a sound local revocation observation.
- `ClearingRequested` suspends automatic loading while retaining bytes and lockfile for inspection. Executed removal hard-disables without deleting forensic material. Restoration requires an explicit local re-enable after a fresh check.

V1 has no automatic `upgrade`. It does not infer release lineage, canonical versions, or publisher-controlled mutable heads.

## Trusted computing base and non-claims

The consumer still relies on:

- consensus honesty and a weak-subjectivity checkpoint in strict mode;
- the locally shipped registry profile, runtime codehash, storage layout, parsers, and cryptographic libraries;
- release signing, build provenance, and local cache integrity; and
- the subjective quality of the Kleros policy and its adjudication.

The verifier does not prove that jurors ruled wisely, that content was continuously available in the past, that a skill is harmless beyond the registry's policy signal, or that arbitrary enriched projections are derived from authenticated inputs.

## Immediate engineering sequence

1. Prove the strict anchor seam with a disposable canary: Gnosis light-client finality → finalized execution `stateRoot` → EIP-1186 account/storage proof → pinned live Classic codehash and `itemCount`.
2. Generate and verify complete Classic snapshots at 1,000 and 10,000 items; measure provider work, compressed transfer size, verification CPU/memory, and RPC limits.
3. Add fixture-backed CAR/UnixFS verification, path and symlink defenses, exact installation, lockfiles, and audit state transitions.
4. Freeze the wire envelope and resource limits from measurements, then integrate the package-manager path into Intendant.

## Deferred work

- Automatic upgrade and TUF-like release lineage.
- LGTCR, T2CR, Token Lists, deterministic projection manifests, and a public adapter API.
- Stateless verified `eth_call`.
- Stake/Permanent Curate or a bounty contract for watchdog incentives.
- Native upstream history/current-set commitments.
- Final product naming.

## Pre-launch inputs

Before production deployment, the owner must approve concrete proposals for the pinned Agent Skills revision, deposits and challenge period, court parameters, governor/governance arrangement, watchdog funding, checkpoint and release-signing policy, snapshot freshness defaults, final MetaEvidence/policy/logo CIDs and manifest, arbitrator proxy/implementation pins, and production registry address/codehash.
