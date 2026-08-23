# Intendhub Verified Snapshot Specification

**Version**: 0.1 (draft)
**Status**: Draft for review — do not freeze before the open items in §11 are resolved.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are to be interpreted as described in RFC 2119.

## 1. Purpose

This specification defines how untrusted data providers serve the complete contents of the Intendhub Skills Registry, and how consumers verify what they receive through a proof mode or an explicit RPC-quorum fallback.

Design goals:

- **Untrusted bulk transport.** Any CDN, indexer, or mirror can serve snapshots at full speed. In a proof-verification mode, a provider cannot forge, alter, substitute, or omit an entry without detection.
- **Provable completeness.** A verified snapshot proves it contains *every* item ever submitted to the registry — including rows that are currently Absent — not merely that each returned item is real.
- **One anchor.** Proof verification reduces to one finalized execution header authenticated by a Gnosis light client or trusted through an explicit RPC-header quorum (§7).
- **O(N) is accepted.** A package index is inherently O(N) to download; the goal is that *chain interaction* stays small and *trust* stays constant-size.

Non-goals: this spec does not define the listing policy (see `listing-policy.md`), submission tooling, or dispute flows. It proves neither historical submission-period availability nor present or future content availability. Those facts do not follow from a descriptor, CID, registry status, snapshot, or storage proof.

## 2. Why this works: registry properties relied upon

The registry is an **unmodified Classic GeneralizedTCR** (Kleros), deployed via the official GTCRFactory on Gnosis Chain. This spec relies on these properties of that contract:

1. `itemList` is an **append-only** array of item IDs. Items are appended exactly once, on first submission; removal or re-request never removes or re-appends. Rejected and removed items remain enumerable, although current Absent status alone does not distinguish those histories.
2. `itemCount()` returns the exact length of `itemList`.
3. The full descriptor bytes of every item are stored in contract storage and returned by `getItemInfo(itemID)`.
4. `itemID = keccak256(descriptor)` — the ID *is* the content hash of the descriptor. A provider therefore cannot substitute descriptor bytes: the client re-hashes them against the proven ID.
5. Item status is stored in contract storage at a computable slot.

Consequence: proving `itemCount` plus every `itemList[i]` plus every status slot, against one state root, proves the **complete** registry. Descriptor bytes need no storage proofs at all — they self-verify against the proven IDs (property 4).

## 3. Registry binding and local verifier profile

A verifier MUST be configured with a local, pinned profile for exactly one registry deployment:

| Parameter | Value |
|---|---|
| `chainId` | 100 (Gnosis) |
| `registry` | `[REGISTRY_ADDRESS]` |
| `expectedRuntimeCodeHash` | `[CODEHASH]` — locally pinned keccak256 of the registry's deployed runtime code |
| `arbitrator` | `[ARBITRATOR_ADDRESS]` (informational) |

The profile, including `expectedRuntimeCodeHash` and the §5 storage-layout constants, comes from local configuration or a pinned software release — never from the provider's snapshot. Clients MUST compare the account proof's code hash with `expectedRuntimeCodeHash` before applying any storage-layout constant. **All slot constants in §5 are properties of this exact bytecode, not of the protocol**; they MUST be re-derived and test-verified against the deployed contract before this spec is frozen.

### 3.1 Descriptor encoding

The descriptor is the RLP encoding of the six policy columns, in policy order (Name, Description, Tree CID, Runtimes, Origin, Reserved), each as a UTF-8 string. Origin is an empty string when unused, and Reserved MUST be an empty string. `itemID = keccak256(descriptor)`. The encoder MUST be byte-compatible with the `@kleros/gtcr-encoder` convention used by Kleros tooling; implementations MUST include cross-implementation test vectors.

### 3.2 Canonical Tree CID syntax

The Tree CID column MUST be a bare, minimally encoded CIDv1 in canonical lowercase base32 form using the DAG-PB codec and a 32-byte SHA-256 multihash. Snapshot verifiers MUST reject CIDv0, noncanonical encodings or varints, other multihash algorithms or digest lengths, uppercase or mixed base encodings, `/ipfs/` or gateway prefixes, path suffixes, IPNS names, and DNSLink references. A CID alone does not reveal whether its DAG-PB root is a UnixFS directory; that semantic check occurs when the root block and complete DAG are fetched in §9.

## 4. Logical snapshot and provisional transport

A snapshot logically contains a registry binding, block anchor, complete ordered row set, account proof, and all storage proofs required by §6. The following JSON envelope and hash-keyed trie-node store are the **current prototype transport**, not a frozen wire format. The 1k/10k benchmarks in §11 gate the final framing, compression, limits, and node encoding.

```json
{
  "version": "0.1",
  "binding": { "chainId": 100, "registry": "0x…" },
  "anchor": {
    "blockNumber": 47912345,
    "blockHash": "0x…",
    "stateRoot": "0x…",
    "timestamp": 1755855600
  },
  "itemCount": 1342,
  "rows": [
    {
      "index": 0,
      "itemID": "0x…",
      "status": 1,
      "descriptor": "0x…rlp bytes…"
    }
  ],
  "proofs": {
    "account": [ "0x…rlp node…", "…" ],
    "nodes": { "<keccak(node)>": "0x…rlp node…" },
    "slots": [
      { "slot": "0x…", "value": "0x…", "path": [ "<nodehash>", "…" ] }
    ]
  }
}
```

- `binding` MUST match the locally pinned profile's `chainId` and `registry`. It carries no trusted code hash or storage layout.
- `rows` MUST contain every index `0 … itemCount-1`, in order, including rows whose current status is `0` (Absent). `status` uses the contract enum: `0` Absent, `1` Registered, `2` RegistrationRequested, `3` ClearingRequested.
- `proofs.account` is the EIP-1186 account proof for `registry` at `anchor.blockNumber`.
- In the prototype transport, `proofs.nodes` is a **deduplicated** trie-node store: each Merkle-Patricia node appears once, keyed by its hash; `slots[].path` lists node hashes root-to-leaf. A 27-item experiment motivates deduplication but is not sufficient to freeze this representation.
- `proofs.slots` MUST cover: the `itemList` length slot; the array slot of every index in `rows`; and the status slot of every item in `rows`.

Content (the skill trees themselves) is NOT part of the logical snapshot; it is fetched on demand by CID (§9). A transport MAY colocate content, but snapshot verification neither proves nor promises that content is available.

## 5. Storage layout constants (to verify against pinned bytecode)

For the pinned GeneralizedTCR runtime code, with `itemList` at slot `L` and the `items` mapping at slot `M` (asserted values from source inspection: `L = 13`, `M = 14` — **MUST be verified against the deployed bytecode and covered by tests before freeze**):

- `itemList` length: storage slot `L`.
- `itemList[i]`: slot `keccak256(uint256(L)) + i`.
- `items[id]` base: `B = keccak256(abi.encode(id, uint256(M)))`. The `data` bytes head occupies slot `B`; `status` occupies slot `B + 1` (low-order byte).

Clients MUST parse `status` from the proven word per the pinned layout and MUST treat any undecodable value as fail-closed (not installable).

## 6. Verification procedure (normative)

A conforming verifier, given a snapshot:

1. **Anchor the header.** Obtain a trusted header for `anchor.blockNumber` via one of the modes in §7 and check `anchor.stateRoot`/`blockHash` against it. Reject snapshots older than the consumer's maximum snapshot age (§8).
2. **Verify the account.** Validate `proofs.account` against `stateRoot`; extract the registry's `storageRoot` and `codeHash`; require `codeHash == localProfile.expectedRuntimeCodeHash`. Reject any attempt by the bundle to supply or override the expected value or storage layout.
3. **Verify the count.** Prove the `itemList` length slot; require it to equal `itemCount` and `rows.length`.
4. **Verify enumeration.** For every `i` in `0 … itemCount-1`, prove `itemList[i]` and require equality with `rows[i].itemID`. Contiguity plus the proven length is what makes the snapshot **complete**: a provider cannot omit, reorder, or invent a row without breaking a proof.
5. **Verify descriptors.** For every row, require `keccak256(descriptor) == itemID`. (No storage proof needed — property 4 of §2.)
6. **Verify statuses.** For every row, prove the status slot and require equality with `rows[i].status`.
7. **Decode and screen.** RLP-decode descriptors; require Reserved to be empty; enforce the syntactic CID rules in §3.2. Directory and DAG validation is deferred to content retrieval (§9).
8. **Result.** The verifier now holds the provably complete registry: every entry ever submitted, its exact descriptor, and its status at the anchored block. In proof modes, the external trust input is the header authentication described in §7; the registry profile remains a locally pinned input.

Verification failures MUST reject the entire snapshot (a provider that fails one proof is not honest about the rest).

## 7. Verification modes

| Mode | Verification source | Trust assumption | Use |
|---|---|---|---|
| **Strict proof** | Gnosis beacon-chain light client authenticates a finalized execution header; §6 verifies the snapshot against its `stateRoot` | Gnosis consensus honesty plus the light client's weak-subjectivity checkpoint | Target mode for unattended fleets |
| **Header-quorum proof** | The same finalized execution header is compared across ≥2 independent RPC providers; §6 verifies the snapshot against its `stateRoot` | Header providers do not collude | Initial snapshot-sync mode while the embedded light client is completed |
| **Direct-RPC quorum** | Complete block-pinned `eth_call`/Multicall results are compared across ≥2 independent RPC providers; no snapshot proofs are accepted | Queried providers do not collude | Fallback for interactive use and small registries |

Requirements and notes:

- Proof bytes and the snapshot MAY come from the same untrusted server; their integrity comes from verification against the anchored `stateRoot`. What MUST be independent is authentication of that anchor. In header-quorum mode, at least one header source MUST be operationally independent of the snapshot provider.
- Strict proof mode requires an embedded or otherwise consumer-verifiable Gnosis light client. Gnosis preset support, checkpoint distribution, and interoperability remain open implementation work (§11). The current candidate is a checkpoint authenticated by the signed release, cached forward after verification, with an operator-supplied override; this is not normative until the checkpoint policy receives pre-launch approval.
- Strict-proof and header-quorum-proof anchors MUST be finalized. Direct-RPC calls MUST be pinned to the same finalized block and compared over the complete logical result, not merely sampled rows.
- Direct-RPC quorum is an explicit RPC-trust fallback. It does not authenticate a CDN snapshot and MUST NOT be labeled proof-verified.

## 8. Status semantics, freshness, and the install predicate

- **Installable** ⇔ the freshly verified status is exactly `1` (Registered). Consumers MUST fail closed on `RegistrationRequested`, `ClearingRequested`, Absent, and any undecodable status. Every `ClearingRequested` item is handled uniformly regardless of requester: suspend new installation and automatic loading or execution by default, retain bytes and lockfile state for inspection, and permit only an explicit local override while pending.
- Immediately before installing or activating a skill, consumers MUST perform a **fresh exact-item check** at the latest finalized block, regardless of snapshot age. In a proof mode this means verifying the account and exact item's status slot against the authenticated header; direct-RPC mode uses the quorum fallback. The checked `itemID` MUST remain bound to the locally decoded descriptor and Tree CID. Snapshots answer *completeness*; the point check answers *freshness*.
- Consumers MUST define a maximum snapshot age (RECOMMENDED: 1 day) after which a new snapshot or delta is required.
- Consumers MUST persist a per-registry finalized-anchor high-water mark and reject an older block number or a conflicting block hash at the same height unless the operator explicitly enters a documented recovery flow. Snapshot age alone does not prevent rollback to a recently finalized but older state. Update, install, activation, and audit all advance or preserve this monotonic anchor state.
- **Lockfile-local revocation.** Absent status does not reveal whether an item was once Registered. A consumer MUST NOT infer removal from an Absent snapshot row alone. If its lockfile records an exact item as previously verified Registered and a fresh authenticated check now reports that same `itemID` as Absent, the consumer SHOULD mark that local installation revoked, disable automatic loading or execution, and retain the bytes for forensic inspection.
- **Audit.** `intend audit` MUST fresh-check every exact `itemID` in the local lockfile. It reports currently Registered entries as current, uniformly quarantines `ClearingRequested` entries, applies the lockfile-local Registered-to-Absent rule above, and fails closed on all other or undecodable states. If a quarantined item later returns to Registered, automatic loading remains suspended until a fresh check and explicit local re-enable. Audit MUST NOT convert unrelated Absent rows into a global revocation list.
- **Incremental updates.** New submissions are detected by `itemCount` growth and verified as in §6 for the new indices. Because old statuses change without changing `itemCount`, a conforming updater MUST re-verify the status sweep (or re-fetch a snapshot) at each refresh; cheaper authenticated deltas require the upstream `historyHead` improvement (§11).

## 9. Content retrieval and verification

- Skill trees are fetched by Tree CID from any source (gateways, peers, or mirrors). Consumers MUST verify content as an authenticated DAG — e.g., via CAR responses / the IPFS trustless-gateway protocol — validating every block and requiring the root DAG-PB node to decode as a UnixFS directory. Every linked CID MUST use a 32-byte SHA-256 multihash and either the DAG-PB or raw codec; other hash algorithms, digest lengths, and codecs reject the tree. **Hashing an individual file in isolation is not sufficient**; path and directory integrity MUST be verified to the root CID.
- Installers MUST write only files present in the verified DAG and MUST sanitize paths (no absolute paths, no `..`, no symlink escapes). The lockfile MUST record at least the exact `itemID`, Tree CID, registry binding, last verified status, and finalized anchor block/hash so later audits can distinguish a local Registered-to-Absent transition from an item that was never Registered.
- CID/DAG verification establishes content integrity. Registry status establishes the policy result for that exact descriptor at a block. Neither establishes availability during the submission period, and installation MUST stop if the content cannot currently be fetched and verified.

## 10. Security considerations

- **Anchor authentication** is the chief implementation hazard: a valid proof only binds data to the supplied root. Authenticate that root through the light client or explicit header quorum; do not treat provider-supplied header fields as self-authenticating (§7).
- **Codehash pinning**: the expected code hash and layout come from the local profile, never the bundle. All layout constants are void against different bytecode (§3, §5).
- **Non-exhaustive point mode**: results served with per-item proofs but without the full enumeration (§6 steps 3–4) prove soundness, not completeness, and MUST be labeled "verified, non-exhaustive."
- **Availability ≠ integrity**: this spec makes a provider's registry projection and fetched DAG verifiable; it does not prove that skill bytes were available during policy review or ensure that anyone will serve them now or later. Currently unfetchable content is uninstallable. Historical review-period availability remains an evidentiary policy question outside this snapshot protocol.
- **Fail closed everywhere**: undecodable statuses, malformed CIDs, failed proofs, oversized trees (policy criterion 9), and unverifiable DAGs all reject.
- **History replay is not a consumer path**: reconstructing the registry from logs requires authenticating every header since deployment (measured on Gnosis: ~14 MB/day of compact proof stream — ~5 GB/year — due to 5-second blocks and saturated blooms). It remains a valid independent-auditor protocol.

## 11. Open items before freeze

1. Verify §5 slot constants and §3.1 encoder byte-compatibility against the deployed bytecode; add cross-implementation test vectors.
2. Benchmark snapshot generation and verification at 1k and 10k synthetic entries, including provider generation time, compressed and uncompressed bytes, verifier CPU and peak memory, inline trie-node handling, absent-slot proofs, malformed-input limits, and CAR-verified installation. Use the results to choose and freeze the transport framing and resource limits.
3. Gnosis light-client availability: upstream Gnosis preset support to an embeddable client (e.g., Helios) or maintain a tested build; define the checkpoint-distribution channel.
4. Upstream feature requests to Kleros Curate V2: (a) `historyHead`-style transition accumulator for cheap authenticated deltas; (b) a current-set commitment root for constant-size completeness proofs. Either would simplify §8's incremental-update rule.
