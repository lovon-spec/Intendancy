> **Supersession notice:** This is historical review material. [RFC 0001: Intendancy V1 Owner Decision](./0001-intendancy-v1-decision.md) is canonical; any conflicting conclusion here is superseded.

# RFC 0001 Review Brief: Intend — Verifiable Onchain Data Egress

**Status:** Pre-RFC; request for architectural review<br>
**Audience:** Claude architecture review, Codex repository/proof review, project owner<br>
**Review mode:** Read-only. Do not edit implementation or normative policy files yet.<br>
**Working name:** `intend` is provisional.

## 1. Requested review

Please challenge this architecture rather than merely elaborating it. Return:

1. Your overall verdict: sound product boundary, useful but over-generalized, or wrong abstraction.
2. The strongest technical or security objections.
3. Any place where the sketch claims more trustlessness, completeness, freshness, or availability than it can deliver.
4. A revised component boundary if you disagree with the proposed one.
5. The smallest coherent V1 for Intendant's secure skill/package retrieval.
6. Which capabilities should be deferred.
7. Concrete decisions that must be frozen before `listing-policy.md`, the snapshot specification, MetaEvidence, encoder, and tests are amended.

Pay particular attention to the questions in section 12.

## 2. Motivation

Intendancy began as a trust layer for Intendant, an agentic development environment that needs to discover and install AI-agent skills and eventually plugins without trusting a marketplace, mutable repository reference, hosted indexer, CDN, RPC, or package publisher.

The Kleros investigation exposed a more general missing layer. Onchain registries may be authoritative, but real applications normally consume convenient JSON produced by trusted infrastructure. Examples include `t2crtokens.eth`, Token Lists, Scout data, address tags, and package indexes. Consumers commonly fetch JSON over HTTPS or through an IPFS gateway and schema-validate it; they do not prove that it is a complete and correct projection of finalized onchain state.

The proposed product boundary is therefore broader than one skills registry:

> `intend` fetches ordinary offchain artifacts derived from onchain state and locally verifies their source, freshness, completeness when the source contract makes completeness provable, and deterministic transformation.

"Reverse oracle" is the product metaphor. The more exact technical description is a light-client-backed verifiable materializer or proof-carrying onchain-to-offchain egress layer.

## 3. Proposed product split

- **`intend`** — open protocol, library, and CLI: consensus light clients, state-proof verification, registry adapters, proof bundles, deterministic projections, and content verification.
- **Intendancy** — the first registry/application profile, initially curating AI-agent skills through an unmodified Classic Kleros GeneralizedTCR on Gnosis Chain. It may also operate mirrors and proof-bundle generation, but clients do not trust those services.
- **Intendant** — the first consuming environment and package-manager integration.

This naming is provisional. The architectural separation is the proposed decision.

## 4. Threat model and non-claims

An untrusted provider may:

- omit, reorder, insert, or mutate registry rows;
- serve stale state or roll a client back;
- publish a projection that applies the wrong filtering, ordering, deduplication, normalization, or transformation;
- serve arbitrary bytes from an HTTPS/IPFS gateway;
- collude with an RPC endpoint;
- correctly prove selected rows while hiding other qualifying rows.

`intend` should detect these attacks to the degree declared by the selected registry adapter and verification mode.

The tool does **not** by itself prove:

- that a subjective registry policy is wise or was adjudicated correctly;
- content availability, past or future;
- completeness for a contract that exposes neither authenticated enumeration nor a current-set commitment;
- that a locally installed weak-subjectivity checkpoint is socially canonical;
- that arbitrary provider-supplied adapter or projection code is safe. Adapters and projections are part of the locally pinned verifier profile.

## 5. Proposed verification pipeline

```text
untrusted provider / CDN / mirror
  -> raw rows + proof bundle + conventional output
  -> embedded chain consensus light client
  -> authenticated finalized execution stateRoot
  -> account/storage proof verifier
  -> locally pinned registry adapter
  -> authenticated raw registry view with a computed capability grade
  -> locally pinned deterministic projection
  -> tokenlist.json / package index / address tags / SQLite / API result
  -> application-specific policy and install decisions
```

The layers must remain separate:

1. **Consensus anchoring** authenticates the finalized state root.
2. **State verification** authenticates accounts, code hashes, and storage.
3. **Registry adaptation** defines decoding, status semantics, and the available enumeration proof.
4. **Projection** deterministically converts authenticated raw state into an application format.
5. **Artifact verification** validates external content-addressed bytes and DAGs.
6. **Application policy** decides whether an authenticated item is safe or installable.
7. **Operational serving** supplies bytes but is not a trust root.

## 6. Verification dimensions

Anchor strength and result completeness are independent dimensions. The verifier, not the provider, computes both.

### 6.1 Anchor modes

- **Consensus:** an embedded proof-of-stake light client authenticates a finalized execution header/state root. Trust assumption: consensus honesty plus a weak-subjectivity checkpoint.
- **Header quorum:** state proofs are checked against a finalized header agreed by independently configured RPC providers.
- **RPC quorum:** explicit block-pinned view calls are compared across independently configured RPC providers. This is convenient but is not a consensus-light-client result.

### 6.2 Result capability grades

- **`historical-complete`:** proves every item ever admitted to the registry's enumerated universe, including tombstones. Example: Classic GTCR's append-only `itemList`; the original Ethereum `ArbitrableTokenList`.
- **`current-set-complete`:** proves the canonical present set against a current-set root/count with specified canonical leaf ordering and tree construction. This need not preserve tombstone history.
- **`sound-only`:** proves that every returned row is genuine at the anchor, but omission remains possible. Example: current LightGeneralizedTCR and PermanentGTCR mappings without authenticated enumeration.
- **`publisher-attested`:** conventional JSON/ENS/HTTPS consumption without registry derivation proofs.

A complete anchor does not upgrade a `sound-only` adapter into a complete result.

## 7. Registry adapter model

An adapter is locally recognized and version-pinned. It must not be executable logic selected by the untrusted provider. An adapter declares:

- chain/network and consensus backend;
- registry address or deployment-binding rules;
- expected runtime code hash, including proxy/implementation verification where relevant;
- storage layout and proof-key derivation;
- item-ID/data binding;
- enumeration mechanism and its precise completeness scope;
- raw status decoding;
- content-reference decoding;
- canonical conformance vectors;
- supported proof types and capability grade.

Initial conformance adapters are proposed:

1. **Classic GeneralizedTCR / Gnosis** — `historical-complete`; used by Intendancy.
2. **Original ArbitrableTokenList / Ethereum** — `historical-complete`; a second adapter proving the abstraction is not skills-specific.
3. **LightGeneralizedTCR / Gnosis** — `sound-only`; included rows can be verified, catalog completeness cannot.

Potential later primitive: a provider supplies all code/account/storage witnesses touched by a view call and `intend` locally executes the EVM against the authenticated state root. This could generalize verified point calls, but it still cannot manufacture an enumerable universe that the contract does not define.

## 8. Proof bundle and deterministic projection

A generic bundle is expected to contain:

```text
format version
adapter/profile ID and version/digest
chain ID and registry binding
finalized block number/hash/stateRoot/time
computed completeness scope
raw item IDs, descriptors, and statuses
deduplicated account/storage proof nodes
optional deterministic projection manifest
optional conventional output and output hash/CID
```

A projection profile declares every input and transformation that affects output:

- qualifying statuses;
- ordering and canonical serialization;
- inclusion/exclusion rules;
- duplicate resolution;
- normalization rules;
- rejected input rows and reasons;
- transformer version/digest;
- authenticated input closure;
- output hash or CID.

External inputs that are not authenticated by the registry proof must be either independently authenticated or explicitly classified as trusted enrichment. A source proof alone does not prove a transformed logo, derived decimal value, semver, or publisher-chosen timestamp.

Compatibility goal: preserve standard output such as `tokenlist.json` and add proof sidecars. Existing clients continue to work; strict clients insert verification immediately after fetching.

## 9. Content retrieval

An IPFS CID makes local verification possible; an ordinary browser `fetch()` through an HTTPS gateway does not itself verify the CID or UnixFS DAG.

For content-addressed artifacts, `intend` should use a trustless gateway/CAR path and validate all blocks and directory/path structure to the root CID. Package installation must additionally reject absolute paths, `..` traversal, and symlink escapes.

Availability remains operational. Multiple providers, mirrors, authors, and archivers improve it, but a valid proof cannot force anyone to serve bytes.

## 10. Intendancy skills/package profile

The proposed V1 source is an unmodified Classic GeneralizedTCR deployed through the official Kleros factory on Gnosis. The descriptor commits to a complete IPFS UnixFS skill-tree CID plus compact metadata. Skill bytes remain offchain and reviewability relies pragmatically on the Kleros/Intendancy ingestion and CDN path.

Proposed package-manager flow:

- `intend update` fetches and verifies a full catalog snapshot or authenticated delta.
- `intend install` resolves a curated release, performs a fresh finalized status check, fetches and verifies the Tree CID, sanitizes paths, and writes a lockfile.
- `intend upgrade` moves only through an authenticated package identity/version/supersession relationship and refuses rollback.
- `intend audit` compares installed CIDs with fresh clearing/removal status.

Installers should fail closed unless the fresh status is exactly `Registered`. A `ClearingRequested` entry is quarantined immediately; an absent entry is not installable.

Open schema blocker: the current draft descriptor has no stable package identity, version, or `supersedes` relationship, so `update` is definable but safe automatic `upgrade` is not. The current speculative Bond Reference column may be the wrong use of scarce schema surface.

Review-period data availability is a separate Kleros policy concern. The current policy draft's claim that availability must never be adjudicated, plus its ingestion-receipt overlay, is not the accepted pragmatic direction. The expected rule is material public retrievability throughout registration/dispute/appeal, with content identity remaining hosting-independent and permanent post-listing availability not guaranteed.

## 11. Upstream registry improvements

Two different commitments solve different problems:

- **`historyHead + transitionCount`** authenticates deltas for a client that already knows a prior head and obtains every intervening transition. It does not give a cheap fresh bootstrap and does not provide data availability.
- **`currentSetRoot + count + canonical tree rules`** gives a constant-size onchain commitment to a full O(N) current snapshot. It does not preserve tombstone history or automatically provide deltas.

For current LGTCR, neither exists. `intend` must report `sound-only`, require expensive authenticated history replay, or refuse a requested complete result until an upstream contract supports it.

## 12. Questions for Claude

1. Is `intend` a coherent general primitive, or is it merely a collection of contract-specific clients hidden behind an adapter interface?
2. Is “verifiable materializer” the correct abstraction? What should the public claim and non-claim be?
3. Is the separation between anchor mode and completeness grade complete and correctly named?
4. What hidden trusted computing base remains in adapter definitions, projection code, checkpoint distribution, proof generation, content decoding, or package resolution?
5. Should V1 locally decode adapter-declared storage proofs, locally execute stateless EVM view calls, or support both? What is the smallest safe choice?
6. Is a provider-generated deduplicated EIP-1186/Merkle-Patricia proof bundle the right transport, or is another witness format materially better?
7. What is the cleanest Gnosis embedded-light-client strategy, given its distinct consensus preset and the lack of turnkey Gnosis support in Helios?
8. Can the package-manager profile safely define identity/version/supersession without introducing a mutable publisher namespace or centralized naming authority?
9. Should every clearing request quarantine a package immediately, or should the profile distinguish trusted emergency requesters despite the additional trust and proof surface?
10. What is the correct pragmatic review-period availability rule that avoids both dark submissions and cure/bait griefing as much as possible?
11. Are deterministic projection manifests sufficient to replace trusted exporters such as Atlas, especially when outputs contain lossy transformations or enrichment?
12. Which proposed components should be removed from V1 to avoid building a generic data framework before proving the Intendant package-manager wedge?
13. Should the project use one foundation RFC plus a dependent Intendancy profile RFC, or a different document split?

## 13. Proposed RFC and implementation sequence

1. Architecture review of this brief by Claude and Codex.
2. Accept a foundation RFC for the core guarantees, adapter contract, bundle envelope, capability taxonomy, and light-client boundary.
3. Accept a dependent Intendancy skills/package RFC covering descriptor schema, review-period DA, install/quarantine/upgrade semantics, and lockfiles.
4. Spike the embedded Gnosis light client, the highest implementation risk.
5. Implement the generic verifier envelope and Classic adapter.
6. Implement the Intendancy package profile and Intendant integration.
7. Add the literal Ethereum T2CR adapter as the second complete adapter.
8. Add LGTCR `sound-only` support and a Token Lists projection.
9. Pursue current-set and transition commitments upstream.

No production policy, MetaEvidence, encoder, contract tests, or snapshot specification should be frozen before the shared schema and proof claims are resolved.
