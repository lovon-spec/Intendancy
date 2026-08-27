# Intendhub Verified Snapshot Specification

**Version**: 0.2
**Status**: Partially frozen. The proof-verification core (§2, §5, §6 steps 0–6)
and the descriptor encoding (§3.1, §3.2) are **FROZEN** on the strength of the
RFC 0001 gate evidence
([Gate 1](spikes/gnosis-anchor-results.md), [Gate 2](spikes/snapshot-bench-results.md),
both reviewer-signed-off) — with ONE named exception inside those sections: the
`metaEvidenceUpdates` **slot-9 constant** (§5), and therefore the concrete slot
value behind §6 step 3b, is **PROVISIONAL** pending review of its evidence
record (the step's LOGIC is frozen; the constant freezes with that record). The
wire framing and final resource-limit values (§4,
§4.1) are **PROVISIONAL** pending the binary-framing spike; the **installer
contract (§9) and the freshness/audit operational values in §8 are PROVISIONAL**
pending closure of the `intend` CLI review and the pre-launch owner decisions
(§11); the deduplicated node store is frozen **in principle only** — its exact
key/path-reference representation freezes with the framing. See §11 for the
authoritative open-item list. Frozen sections change only through a new reviewed
revision.

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
| `chainId` | 100 (Gnosis) — authenticated via `eth_chainId` against every source |
| `genesisHash` | The chain's genesis block hash (or the pinned checkpoint for the anchor mode in use) — pinned chain identity, verified against every anchor source and proof RPC by IDENTITY comparison of the reported hash. The genesis hash is NOT recomputed from header fields: no genesis field is consumed, and on Gnosis recomputation is impossible anyway (AuRa-era legacy sealing, §7) — field-consuming ANCHOR headers, by contrast, are always recomputed (§7) |
| `registry` | `[REGISTRY_ADDRESS]` — pinned at production deployment |
| `expectedRuntimeCodeHash` | `[CODEHASH]` — locally pinned keccak256 of the registry's deployed runtime code. The current official-factory generation deploys runtime code hashing to `0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d` (verified against the Gate 1 canary and two fresh Gate 2 factory deployments); the production value is pinned from the actual deployment receipt and MUST equal what the factory deployed |
| `arbitrator` | `[ARBITRATOR_ADDRESS]` (informational) |
| `registrationMetaEvidence` / `clearingMetaEvidence` | **MANDATORY** pins of the deployment MetaEvidence/policy references. Trust model: the profile is a locally authenticated release/deployment manifest that pins the factory deployment receipt/events, the registry address, its runtime codehash, and the TWO INITIAL MetaEvidence references those deployment events declared; §6 step 3b's `metaEvidenceUpdates == 0` proof then establishes that no later policy was ever declared — so the pinned references ARE the policy every verdict was judged under. Values must be nonempty canonical references; the production checklist (§11) fills them from the deployment receipt |

The profile, including `expectedRuntimeCodeHash` and the §5 storage-layout constants, comes from local configuration or a pinned software release — never from the provider's snapshot. Clients MUST compare the account proof's code hash with `expectedRuntimeCodeHash` before applying any storage-layout constant, and MUST perform the §6 step-0 identity comparisons before any proof work. **All slot constants in §5 are properties of this exact bytecode, not of the protocol**; they are frozen against the codehash above (§5) and are void against any other bytecode.

Two distinct mechanisms defend the account identity, and they are not redundant with each other. The **registry-key pin** (step 0's `binding.registry` equality plus verifying the account proof under that exact address key) is what prevents ADDRESS substitution: under an authentic state root a provider can construct an entirely honest proof about some other account — e.g. prove that a funded EOA's `itemList` slot is empty and present it as an empty catalog — and every MPT check passes; the registry pin alone rejects it. The **proven-codehash equality** separately binds the BYTECODE at the pinned address: it is what makes the §5 storage-layout constants applicable at all, and it rejects a different contract standing at the expected address (misconfigured pin, redeployment, metamorphic code). Both attacks are maintained test fixtures in the gate/CLI suites.

### 3.1 Descriptor encoding

**FROZEN.** The descriptor is the RLP encoding of the six policy columns, in policy order (Name, Description, Tree CID, Runtimes, Origin, Reserved), each as a UTF-8 string — an RLP list of exactly six string items, nothing more (no trailing bytes; decoders MUST use exact-length decoding). Origin is an empty string when unused, and Reserved MUST be an empty string. `itemID = keccak256(descriptor)`. Implementations MUST ship cross-implementation test vectors; the reference vectors live in the Gate 2 crate (`spikes/snapshot-bench/fixtures/descriptor-vectors.json`, generated by the repo frontend's viem encoder and byte-matched in both directions by the Rust implementation, including Unicode, empty-column, and policy-invalid-but-codec-valid cases). The evidence base is therefore viem/frontend ↔ Rust; a vector generated by the official `@kleros/gtcr-encoder` package would widen it and MAY be added, but no compatibility claim about that package is made here.

### 3.2 Canonical Tree CID syntax

The Tree CID column MUST be a bare, minimally encoded CIDv1 in canonical lowercase base32 form using the DAG-PB codec and a 32-byte SHA-256 multihash. Snapshot verifiers MUST reject CIDv0, noncanonical encodings or varints, other multihash algorithms or digest lengths, uppercase or mixed base encodings, `/ipfs/` or gateway prefixes, path suffixes, IPNS names, and DNSLink references. A CID alone does not reveal whether its DAG-PB root is a UnixFS directory; that semantic check occurs when the root block and complete DAG are fetched in §9.

## 4. Logical snapshot and provisional transport

A snapshot logically contains a registry binding, block anchor (number, hash, AND state root — all three), complete ordered row set, account proof (with the claimed account fields it proves), and all storage proofs required by §6. **Node-store deduplication is adopted in PRINCIPLE** on Gate 2 evidence — at 10,000 items, node-payload deduplication is 7.7× over naive per-key proofs and the full dictionary (payloads + 32-byte keys + path references) is 4.06× (11.50 MB vs 46.64 MB), with full verification in ~140 ms / ~69 MB peak RSS — but the EXACT representation (hash-key encoding, path-reference form) is deliberately NOT frozen here: Gate 2 deferred it to the framing spike, and it freezes with §11 item 1. The shape below is what the current debug encoding uses.

The JSON envelope shown is the **debug encoding**, PROVISIONAL as a wire format: hex-in-JSON measures 31.97 MB raw / 13.60 MB gzip at 10k items and tops out around ~80k items under a 256 MiB decoded cap. The canonical wire format will be a length-prefixed binary framing (or CBOR) with a REQUIRED compressed content-encoding, decided by the framing spike (§11); no binary sizes are quoted here because none have been measured.

```json
{
  "version": "0.2",
  "binding": { "chainId": 100, "registry": "0x…" },
  "anchor": {
    "blockNumber": 47912345,
    "blockHash": "0x…",
    "stateRoot": "0x…"
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
    "accountFields": {
      "nonce": 1, "balance": "0x0",
      "storageRoot": "0x…", "codeHash": "0x…"
    },
    "account": [ "0x…rlp node…", "…" ],
    "nodes": { "<keccak(node)>": "0x…rlp node…" },
    "slots": [
      { "slot": "0x…", "value": "0x…", "path": [ "<nodehash>", "…" ] }
    ]
  }
}
```

- `version` names the envelope revision; verifiers MUST reject a version they do not implement (§6 step 0).
- `binding` MUST match the locally pinned profile's `chainId` and `registry`. It carries no trusted code hash or storage layout.
- `anchor` MUST carry `blockNumber`, `blockHash`, and `stateRoot`; §6 compares all three against the authenticated header. Height plus state root alone is not a complete anchor. The anchor deliberately carries NO timestamp: header TIME comes only from the authenticated header itself (§7) — a provider-claimed timestamp would otherwise leak into freshness decisions.
- `proofs.accountFields` are the CLAIMED account fields (nonce, balance, storageRoot, codeHash) the account proof proves wholesale; a lie in any field fails the MPT check (§6 step 2).
- `rows` MUST contain every index `0 … itemCount-1`, in order, including rows whose current status is `0` (Absent). `status` uses the contract enum: `0` Absent, `1` Registered, `2` RegistrationRequested, `3` ClearingRequested.
- `proofs.account` is the EIP-1186 account proof for `registry` at `anchor.blockNumber`.
- `proofs.nodes` is the **deduplicated** trie-node store: each Merkle-Patricia node appears once, keyed by its keccak256 hash; `slots[].path` lists node hashes root-to-leaf. Verifiers MUST check every store entry's key against the hash of its bytes.
- `proofs.slots` MUST cover: the `itemList` length slot; the `metaEvidenceUpdates` slot (§6 step 3b); the array slot of every index in `rows`; and the status slot of every item in `rows`.

Content (the skill trees themselves) is NOT part of the logical snapshot; it is fetched on demand by CID (§9). A transport MAY colocate content, but snapshot verification neither proves nor promises that content is available.

### 4.1 Resource limits — mechanisms normative, values provisional

Snapshots are untrusted input; the following limit MECHANISMS are normative and
each is exercised by an adversarial rejection test in the Gate 2 reference
implementation:

- **Bounded input at the boundary.** Byte caps MUST be enforced at the
  APPLICATION layer before over-cap input is accumulated: readers stream
  through a take-style bound so at most cap + 1 bytes are ever consumed into
  (or retained by) the application's buffer, regardless of the source's size,
  with the gzip magic sniffed first to select the applicable cap. The 2-byte
  sniff read is itself bounded by the LARGER configured cap + 1, so a
  degenerate sub-2-byte cap still bounds the very first read (at most
  max-cap + 1 bytes in that corner). Stated honestly, this contract governs
  application-layer allocation and consumption accounting — NOT the HTTP
  stack's internals: below the reader, a transport buffers response bytes
  ahead of consumption within its own configuration-bounded buffers (an
  HTTP/2 connection can queue MULTIPLE data frames up to its flow-control
  windows), and that overhead MUST be response-size-independent, with the
  bounding components and limits stated from the LOCKED dependency set or
  explicitly configured. The reference implementation consumes every HTTP
  body as an `io::Read` under `Read::take` (counting-reader tests prove the
  reader-layer consumption bound) over the stack locked by its Cargo.lock —
  reqwest 0.12.28, hyper 1.11.0, h2 0.4.18: for HTTP/1, hyper's read buffer
  is bounded at 417,792 bytes (`DEFAULT_MAX_BUFFER_SIZE`, 8 KiB + 100 ×
  4 KiB) per connection; for HTTP/2, the client EXPLICITLY configures a
  1 MiB stream window, 2 MiB connection window, and 16 KiB max frame, which
  bound the queued frames. These are PER-CONNECTION / PER-STREAM component
  bounds, not a process-wide ceiling: aggregate transport memory scales with
  the number of live and idle-pooled connections and with caller concurrency
  (the reference CLI's command paths are sequential, which keeps practical
  concurrency narrow, but the transport type itself enforces no process-wide
  cap). The blocking-reader adapter additionally retains one in-flight
  `Bytes` chunk, and TLS and kernel socket buffers are platform-managed and
  not numerically established here. None of that overhead is ever appended
  to the application buffer beyond the cap. Decompression MUST be
  bounded independently (a decoded-bytes ceiling stops gzip bombs); the LOGICAL
  RETAINED LENGTH is then bounded per buffer — compressed input ≤ compressed
  cap + 1, decoded buffer ≤ decoded cap + 1 (heap CAPACITY may exceed the
  logical length by the allocator's growth overhead — `Vec` over-allocates;
  the bound is on bytes retained, not on the allocator's rounding). Cap arithmetic MUST NOT under- or overflow for degenerate
  configured values (zero, tiny, maximal); saturating arithmetic with an early
  over-cap rejection of already-read bytes satisfies this.
- **Structural caps, each with its own enforcement branch**: itemCount; row
  count; slot-proof count; node-store entry count; per-node byte size applying to
  BOTH account-proof and storage nodes; aggregate node-store bytes; and per-proof
  path length for BOTH the account path and each storage path (secure-trie keys
  are 64 nibbles, so an honest path never exceeds 65 nodes).
- **Fail closed**: exceeding any bound rejects the snapshot with a typed error.

The concrete VALUES are provisional working values pending the binary-framing
decision (§11) and freeze together with it:

| Limit | Working value |
|---|---|
| Compressed input | 64 MiB |
| Decoded envelope | 256 MiB |
| Per MPT node | 16 KiB |
| Node-store entries | 8,000,000 |
| Node-store aggregate | 192 MiB |
| Items / rows | 1,000,000 (verifier sanity cap — NOT a JSON-transport claim; JSON tops out ≈80k at the decoded cap) |
| Proof path | 66 nodes |

## 5. Storage layout constants (frozen against the pinned bytecode)

**FROZEN** for runtime codehash `0x5a6cf793…5d7d` (§3): `itemList` at slot `L = 13`, `items` mapping at slot `M = 14` — verified empirically against the deployed bytecode: Gate 1 proved the layout against a live canary registry of that codehash, and Gate 2 proved it exhaustively (the length slot, every `itemList[i]` slot, and every status slot) against freshly factory-deployed registries at 1k and 10k scale. Any other codehash voids these constants (§3).

- `itemList` length: storage slot `L`.
- `itemList[i]`: slot `keccak256(uint256(L)) + i`.
- `items[id]` base: `B = keccak256(abi.encode(id, uint256(M)))`. The `data` bytes head occupies slot `B`; `status` occupies slot `B + 1` (low-order byte).
- `metaEvidenceUpdates`: storage slot `9` — **PROVISIONAL pending review of its evidence record** (`docs/spikes/meta-evidence-slot-evidence.md`: an ASSERTING reproducible governor-update probe — codehash recomputed locally, getter and slot 9 moving 0→1→2 in lockstep with every other SAMPLED slot (0..16) proven unchanged, receipts in a digest-recorded sealed transcript — plus the verified-source pin of the LOAD-BEARING no-reset property, re-retrieved and hash-asserted by the probe itself, without which a `== 0` proof would not mean "never updated"), unlike slots 13/14 whose freeze rests on the signed Gate 1/2 records. It freezes when that record is reviewer-accepted; this is the one exception to the §5 freeze noted in the header.

Clients MUST require the entire proven status word to decode as a value in `0..=3` and MUST treat any other value as fail-closed (not installable).

**Zero-valued slots have two provable forms.** A real Ethereum trie deletes a slot on `SSTORE 0`, so a zero value (e.g. the status slot of an item whose removal executed) is proven by an MPT **exclusion proof**; verifiers MUST accept this form. Synthetic tries (observed: anvil's fork mode, used by the reference test environment) instead retain an explicit leaf whose payload is `RLP(0x80)`; verifiers MAY additionally accept that form. Both prove "the slot is zero" against the committed root, and neither form can prove a nonzero value, so accepting both loses no soundness. Evidence, precisely: the Gate 2 bench fixture covers the explicit-zero-LEAF form for an Absent status slot (it was exercising this branch that surfaced the discrepancy) and covers the true exclusion form via the EOA empty-trie `itemList`-length proof; the `intend` CLI's offline suite additionally covers a true exclusion proof for an ABSENT STATUS slot over a real in-memory trie.

## 6. Verification procedure (normative)

A conforming verifier, given a snapshot:

0. **Bind the identity before any proof.** Read the input under the §4.1 bounds.
   Require the envelope `version` to be an implemented revision, and
   `binding.chainId`/`binding.registry` to equal the pinned profile exactly.
   Nothing in the snapshot names its own trust; a mismatch rejects before any
   proof bytes are examined.
1. **Anchor the header.** Obtain a trusted header for `anchor.blockNumber` via one of the modes in §7 and check ALL THREE of `anchor.blockNumber`, `anchor.blockHash`, and `anchor.stateRoot` against it exactly. Reject snapshots older than the consumer's maximum snapshot age (§8).
2. **Verify the account.** Validate `proofs.account` against `stateRoot`; extract the registry's `storageRoot` and `codeHash`; require `codeHash == localProfile.expectedRuntimeCodeHash` (the bytecode/layout binding; the registry-KEY pin in step 0 is what defeats address substitution — §3). Reject any attempt by the bundle to supply or override the expected value or storage layout.
3. **Verify the count, and the policy immutability invariant (step 3b).**
   Prove the `itemList` length slot; require it to equal `itemCount` and
   `rows.length`. Then (owner decision — V1 pins ONE immutable policy per
   registry) prove the `metaEvidenceUpdates` slot and require ZERO: a nonzero
   counter means the registry's policy was changed after deployment, the
   verdicts' policy basis is no longer the pinned deployment policy, and the
   snapshot MUST be rejected. Changing policy means deploying a new registry.
4. **Verify enumeration.** For every `i` in `0 … itemCount-1`, prove `itemList[i]` and require equality with `rows[i].itemID`. Contiguity plus the proven length is what makes the snapshot **complete**: a provider cannot omit, reorder, or invent a row without breaking a proof.
5. **Verify descriptors.** For every row, require `keccak256(descriptor) == itemID`. (No storage proof needed — property 4 of §2.)
6. **Verify statuses.** For every row, prove the status slot and require equality with `rows[i].status`, accepting zero-valued slots in the forms §5 defines; require the whole proven word to decode per §5.
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

- **Chain identity is part of every anchor.** Every anchor source AND every proof
  RPC MUST be authenticated against the pinned chain before use: `eth_chainId`
  MUST equal the profile's `chainId`, and the source's genesis block hash (or a
  pinned checkpoint hash for the mode in use) MUST equal the profile's pin. A
  source on the wrong chain fails closed regardless of header agreement.
- **Header time is authenticated, bounded, and quorum-checked.** Implementations
  MUST recompute the hash of every header whose FIELDS they consume (anchor
  headers: stateRoot, timestamp) and require it to equal the reported hash — a
  source cannot attach a fabricated timestamp to an agreed hash. The
  chain-identity genesis check is a pure hash-to-pin comparison and consumes no
  fields; recomputation neither applies nor is possible there on Gnosis, whose
  AuRa-era headers (genesis included) use legacy sealing outside the standard
  header encoding. Implementations MUST further require timestamp agreement
  across sources for the agreed header, MUST bound FUTURE skew against a fresh
  local clock (a header from the future fails closed rather than counting as
  age zero), and MUST compute all freshness ages from the authenticated header
  timestamp — never from a provider-claimed field.
- Quorum sources MUST be pairwise distinct after URL normalization (scheme,
  lowercased host with loopback aliases folded, port) and SHOULD belong to
  distinct operators/trust domains; distinct URL strings alone are not
  independence.
- Proof bytes and the snapshot MAY come from the same untrusted server; their integrity comes from verification against the anchored `stateRoot`. What MUST be independent is authentication of that anchor. In header-quorum mode, at least one header source MUST be operationally independent of the snapshot provider.
- Strict proof mode requires an embedded or otherwise consumer-verifiable Gnosis light client. Gnosis preset support, checkpoint distribution, and interoperability remain open implementation work (§11). The current candidate is a checkpoint authenticated by the signed release, cached forward after verification, with an operator-supplied override; this is not normative until the checkpoint policy receives pre-launch approval.
- Strict-proof and header-quorum-proof anchors MUST be finalized. Direct-RPC calls MUST be pinned to the same finalized block and compared over the complete logical result, not merely sampled rows.
- Direct-RPC quorum is an explicit RPC-trust fallback. It does not authenticate a CDN snapshot and MUST NOT be labeled proof-verified.

## 8. Status semantics, freshness, and the install predicate

- **Installable** ⇔ the freshly verified status is exactly `1` (Registered). Consumers MUST fail closed on `RegistrationRequested`, `ClearingRequested`, Absent, and any undecodable status. Every `ClearingRequested` item is handled uniformly regardless of requester: suspend new installation and automatic loading or execution by default, retain bytes and lockfile state for inspection, and permit only an explicit local override while pending.
- Immediately before installing or activating a skill, consumers MUST perform a **fresh exact-item check** at the latest finalized block, regardless of snapshot age. NAME-based selection MUST additionally establish uniqueness AT THAT FRESH ANCHOR, not merely in the saved catalog (the listing policy does not require unique names): prove the registry's current `itemCount` equals the verified catalog's — descriptors are immutable, so count equality proves the catalog's same-name candidate set complete — then freshly prove every candidate's status and require exactly one Registered; otherwise refuse and direct the user to an exact-itemID install. In a proof mode this means verifying the account, the exact item's status slot, AND the `metaEvidenceUpdates == 0` invariant (§6 step 3b — the policy could have been changed between the snapshot's anchor and now) against the authenticated header; direct-RPC mode uses the quorum fallback. The checked `itemID` MUST remain bound to the locally decoded descriptor and Tree CID. Snapshots answer *completeness*; the point check answers *freshness*.
- Consumers MUST define a maximum snapshot age, measured against the AUTHENTICATED header timestamp (§7), after which a new snapshot or delta is required. The concrete value is a pre-launch owner decision (§11); implementations ship a conservative default (current working default: 1 day) clearly labeled provisional.
- Consumers MUST persist a per-registry finalized-anchor high-water mark — keyed by chain identity THROUGH the genesis/checkpoint hash plus the registry, so a same-chain-id fork keeps independent marks — and reject an older block number or a conflicting block hash at the same height unless the operator explicitly enters a documented recovery flow. Snapshot age alone does not prevent rollback to a recently finalized but older state. Update, install, activation, and audit all advance or preserve this monotonic anchor state.
- **Lockfile-local revocation.** Absent status does not reveal whether an item was once Registered. A consumer MUST NOT infer removal from an Absent snapshot row alone. If its lockfile records an exact item as previously verified Registered and a fresh authenticated check now reports that same `itemID` as Absent, the consumer SHOULD mark that local installation revoked, disable automatic loading or execution, and retain the bytes for forensic inspection.
- **Audit.** `intend audit` MUST fresh-check every exact `itemID` in the local lockfile AND verify the installed bytes locally: the exact file set (no extras, none missing), sizes, and per-file digests against the lockfile — a divergence is a fail-closed state of its own. It reports currently Registered, locally intact entries as current, uniformly quarantines `ClearingRequested` entries, applies the lockfile-local Registered-to-Absent rule above, and fails closed on all other or undecodable states. Quarantine and revocation are STICKY: an item that later returns to Registered does NOT auto-clear — automatic loading remains suspended until a fresh check and an EXPLICIT local re-enable transition. Audit MUST NOT convert unrelated Absent rows into a global revocation list.
- **Local-state context binding.** Persisted LOCAL verification state — catalog metadata and lockfile entries — MUST bind the full local deployment/profile identity (chain id, genesis/checkpoint hash, registry, expected runtime codehash, the pinned MetaEvidence references, and any test-mode flag; the reference implementation hashes these into a versioned deployment-context id), and consumers MUST refuse to display, resolve from, or transition state verified under a different context. This is a LOCAL rule only: the provider snapshot's `binding` remains `chainId` + `registry` as specified in §4 — providers are not asked to know a consumer's profile.
- **Incremental updates.** New submissions are detected by `itemCount` growth and verified as in §6 for the new indices. Because old statuses change without changing `itemCount`, a conforming updater MUST re-verify the status sweep (or re-fetch a snapshot) at each refresh; cheaper authenticated deltas require the upstream `historyHead` improvement (§11).

## 9. Content retrieval and verification

The installer CONTRACT below is **PROVISIONAL** — validated by the Gate 2
prototype and its adversarial suite, and being hardened through the `intend` CLI
review, but not frozen until that review closes. The reference implementation's
install transaction now journals the FULL verified manifest before publishing
(non-replacing: journaling never overwrites a prior record), publishes through a
once-bound parent directory fd with identity spot-checks (the parent pathname
is re-compared against the held fd before publish, before the publish rename,
and before the finalizing lockfile write — a retargeted pathname fails closed;
error cleanup is likewise fd-relative), and establishes power-loss ordering
(file, directory, and parent fsyncs before the finalizing lockfile save; the
enable finalizer re-binds the destination and re-fsyncs its parent BEFORE
clearing a pending journal, closing the crash window between the publish
rename and the parent fsync). Crash behavior is exercised by child-process
kill tests at four exact boundaries. Recovery semantics, split exactly by
boundary (round-7): (1) a crash BEFORE the journal's atomic rename leaves NO
record and NO content — recovery is an ordinary retry; (2) a crash after the
journal is durable but BEFORE the publish rename leaves a fail-closed pending
journal whose removal (to reinstall at that path) is a deliberate MANUAL
edit — and a crash during staged extraction may additionally orphan one
private `.intend-stage-*` directory, safe to delete when no installer is
running; neither state is auto-repaired; (3) a crash AFTER the publish rename
(the rename-before-parent-fsync window included) or during the finalizing
save recovers through audit + the explicit enable finalizer, which
re-establishes destination durability before clearing the journal. The parser
profile/stack selection remains open (kubo interop vectors for the bounded
UnixFS-basic profile exist in `cli/fixtures/kubo/` — the DEFAULT vector,
including its chunked 600 KB File, is proven in both directions; the METADATA
vector is consumer-side only — regenerated deterministically across
environments; the maintained-stack alternative is still un-evaluated).

**Named production blockers within this section** (explicitly OUT of the current
implementation's claims): (a) the local **audit walk is not race-free** — node
types are checked with lstat and never knowingly followed, but directories are
subsequently opened by path and files by open(2), so a concurrently modifying
LOCAL attacker can race the checks; the integrity verdict is exact against
non-concurrent modification and advisory under an active local adversary until
fd-relative secure traversal lands. (b) The install-target **bind is
canonicalize-then-open** — narrowed by O_NOFOLLOW and a dev/ino cross-check,
not a fully race-free resolution. (c) A same-user process can **replace the
published FINAL child name** inside the (identity-verified) parent after
publication — the parent-identity spot checks do not cover the child entry
itself; later audits fail closed on the divergence, but the window exists.
All three are implementation-hardening items, not protocol changes.

- Skill trees are fetched by Tree CID from any source (gateways, peers, or mirrors). Consumers MUST verify content as an authenticated DAG — e.g., via CAR responses / the IPFS trustless-gateway protocol — validating every block and requiring the root DAG-PB node to decode as a UnixFS directory. Every linked CID MUST use a 32-byte SHA-256 multihash and either the DAG-PB or raw codec; other hash algorithms, digest lengths, and codecs reject the tree. The reference implementation additionally requires **CIDv1 for every linked CID** (a CIDv0 dag-pb child rejects) — whether V1 admits CIDv0 children is a flagged PRE-FREEZE decision: the listing policy is currently silent on link versions while requiring a CIDv1 root, and kubo's `--cid-version 1` emits v1 links throughout, so requiring v1 everywhere is the recommended resolution. **Hashing an individual file in isolation is not sufficient**; path and directory integrity MUST be verified to the root CID.
- **The expected Tree CID is the sole root authority.** It comes from the
  verified descriptor; a CAR's own root claim, a gateway's response framing, or
  any other transport metadata MUST NOT be treated as naming the root. Content
  under a different root is refused before anything is written.
- **Every block is verified by rehash**, wherever the bytes came from: a block's
  hash equals its CID's digest, checked at read time AND on first use — a
  CID-keyed map is data, not evidence. Every block in the transported set MUST be
  reachable from the root (no smuggled extras); a block MAY be referenced by any
  number of links (shared subtrees and duplicate file contents are legitimate)
  but appears in the transport once.
- **Preflight fully, then publish atomically.** The complete DAG MUST be
  validated — structure, names, bounds — with NO filesystem writes on any failure
  path. Extraction stages into a fresh private directory and publishes with one
  atomic NO-REPLACE rename; an install that fails with a RETURNED error
  leaves no destination and no staging residue (a hard CRASH during staged
  extraction may orphan one private stage directory — see the recovery
  semantics above). The destination MUST NOT pre-exist in any form: file,
  directory, or symlink, checked without following (a dangling symlink is still
  an occupant), after resolving the destination's parent to its canonical path
  ONCE — the same binding is used for the pre-publish journal, the staging, and
  the publish rename (the reference implementation holds the bound parent as a
  directory fd so a retargeted parent PATH cannot redirect the publish).
  Durability ordering: staged file contents, staged directories, and — after
  the rename — the parent directory are fsynced before the finalizing lockfile
  write, so a power loss never leaves a finalized record pointing at
  non-durable content.
- **Bounds during preflight**, enforced as the plan grows (per entry, so they
  hold within a single directory): tree depth; file and directory counts; and the
  listing policy's aggregate tree cap counted over MATERIALIZED bytes — a shared
  block counts once per file it becomes, so sharing cannot smuggle an
  over-budget tree under the cap.
- Installers MUST write only files present in the verified DAG and MUST sanitize paths: reject EMPTY entry names (a dag-pb link's Name field may be absent and decodes as empty — an empty-named directory entry would join to its parent path and collapse a directory level, so it MUST fail preflight; file-CHUNK links inside a File node are the one place empty names are required), absolute paths, `.`, `..`, separators and NUL inside entry names, and duplicate names within a directory. Trees contain directories and regular files ONLY (listing-policy v2.1): ANY symlink node in the DAG rejects the tree outright — not merely escaping symlinks — and installers never create symlinks. Names that are bytewise distinct MAY still coalesce on case- or normalization-insensitive filesystems; installers MUST detect the resulting collisions and fail closed rather than silently overwrite.
- The lockfile MUST record at least the exact `itemID`, Tree CID, the FULL local deployment/profile identity (per §8's local-state context binding — not merely chain + registry), last verified status, finalized anchor block NUMBER AND HASH, and per-file digests, so later audits can distinguish a local Registered-to-Absent transition from an item that was never Registered. Fields whose values are not yet verified MUST be recorded as absent, never as placeholders.
- CID/DAG verification establishes content integrity. Registry status establishes the policy result for that exact descriptor at a block. Neither establishes availability during the submission period, and installation MUST stop if the content cannot currently be fetched and verified.

## 10. Security considerations

- **Anchor authentication** is the chief implementation hazard: a valid proof only binds data to the supplied root. Authenticate that root through the light client or explicit header quorum; do not treat provider-supplied header fields as self-authenticating (§7).
- **Wrong-account proofs (the EOA empty-catalog attack)**: under an authentic root, a provider can honestly prove the empty storage of an account that is not the registry and present it as an empty (or different) catalog. Every proof verifies; the REGISTRY-KEY pin (§6 step 0 + the address key of the account proof) is what rejects address substitution. The proven-codehash equality (step 2b) is a SEPARATE mechanism binding the bytecode and therefore the §5 layout at the pinned address (§3). A verifier needs both, for different attacks.
- **Codehash pinning**: the expected code hash and layout come from the local profile, never the bundle. All layout constants are void against different bytecode (§3, §5).
- **Resource exhaustion**: snapshots and CARs are attacker-sized inputs. The §4.1 mechanisms (application-layer bounds before accumulation plus bounded transport components, bounded decompression, per-structure caps with overflow-safe arithmetic) and §9's preflight bounds (materialized-byte accounting included) are normative, not advisory.
- **Non-exhaustive point mode**: results served with per-item proofs but without the full enumeration (§6 steps 3–4) prove soundness, not completeness, and MUST be labeled "verified, non-exhaustive."
- **Availability ≠ integrity**: this spec makes a provider's registry projection and fetched DAG verifiable; it does not prove that skill bytes were available during policy review or ensure that anyone will serve them now or later. Currently unfetchable content is uninstallable. Historical review-period availability remains an evidentiary policy question outside this snapshot protocol.
- **Fail closed everywhere**: undecodable statuses, malformed CIDs, failed proofs, oversized trees (policy criterion 9), and unverifiable DAGs all reject.
- **History replay is not a consumer path**: reconstructing the registry from logs requires authenticating every header since deployment (measured on Gnosis: ~14 MB/day of compact proof stream — ~5 GB/year — due to 5-second blocks and saturated blooms). It remains a valid independent-auditor protocol.

## 11. Open items

Resolved by the RFC 0001 gates (evidence: `docs/spikes/gnosis-anchor-results.md`,
`docs/spikes/snapshot-bench-results.md`, both reviewer-signed-off):

- ~~Verify §5 slot constants and §3.1 encoder byte-compatibility; add cross-implementation vectors.~~ Done — §5 constants verified against the deployed bytecode at three deployments (Gate 1 canary + two Gate 2 fresh deploys, codehash `0x5a6cf793…5d7d`); descriptor vectors generated by the frontend's viem encoder and byte-matched by the Rust implementation. §3.1 and §5 are frozen.
- ~~Benchmark 1k/10k generation and verification, absent-slot proofs, malformed-input limits, CAR-verified installation.~~ Done — measurements, adversarial suites (69 tests in the Gate 2 crate `spikes/snapshot-bench` — the count reproducible as `cargo test --locked` there — incl. the wrong-account fixture, both zero-slot proof forms, per-branch limit rejections, install tamper cases), and the freeze recommendations this revision applies. The Gate 2 feasibility numbers: 10k full verify ≈140 ms / ≈69 MB; generation is proof-harvest-dominated.

Still open before the spec can claim 1.0:

1. **Binary wire framing + final limit values.** Choose the canonical
   length-prefixed binary (or CBOR) framing with REQUIRED compression, measure
   it, and freeze §4's format and §4.1's values together. Until then JSON is the
   debug encoding and the §4.1 values are working values. (Framing spike.)
2. **Production installer parser and contract closure.** The bounded
   "UnixFS-basic" profile now has **kubo interop vectors** — the DEFAULT
   vector, including its chunked 600 KB File, proven in BOTH directions; the
   metadata-bearing mode+mtime+exec-bit vector CONSUMER-side only
   (`cli/fixtures/kubo/`, both regenerated deterministically by
   `cli/tools/gen-kubo-vectors.sh`), and the Gate 2 residual list (varint
   canonicality, duplicate protobuf fields, filesystem name coalescing, staging
   privacy) is addressed in the `intend` CLI port, as are the round-3
   transaction-boundary findings (full-manifest pending journal, non-replacing
   journaling, no-replace fd-relative publication with identity spot-checks,
   fsync ordering incl. the enable finalizer's durability re-fsync,
   interprocess serialization, child-process crash tests). The profile remains
   a deliberately NARROW kubo-interop profile, not general UnixFS conformance.
   Remaining before §9 freezes: the named race-hardening blockers in §9 (audit
   traversal, bind resolution, final-child-name replacement),
   broader-than-one-producer compatibility evidence, the maintained-stack
   alternative evaluation, and the **CIDv0-children owner decision** (§9: the
   implementation requires CIDv1 for every linked CID; the recommendation to
   the owner is to make that the policy rule, matching kubo `--cid-version 1`
   output). (The former owner-decision dependencies — policy binding and tree
   node types — are DECIDED, items 6/7.)
3. **Gnosis light-client productization.** Gate 1 proved feasibility with no
   fork (helios-consensus-core's public `ConsensusSpec` trait; Gnosis injected as
   data); remaining: upstream the Gnosis preset (parametrize seconds-per-slot),
   or maintain the tested build, and define the checkpoint-distribution channel
   (pre-launch owner decision, per §7).
4. **Upstream feature requests to Kleros Curate V2**: (a) `historyHead`-style
   transition accumulator for cheap authenticated deltas; (b) a current-set
   commitment root for constant-size completeness proofs. Either would simplify
   §8's incremental-update rule. Not yet filed.
5. **Production profile values.** `[REGISTRY_ADDRESS]`, the production
   `[CODEHASH]` (from the actual factory deployment receipt), the arbitrator
   address, the chain `genesisHash` (and/or the light-client checkpoint), and
   the two deployment MetaEvidence/policy references (from the deployment
   events in the same receipt) are pinned at deployment (owner-gated, per the
   repository's pre-launch rules). The maximum-snapshot-age value (§8) is
   likewise pinned pre-launch.
6. ~~OWNER DECISION — policy/MetaEvidence binding invariant.~~ **DECIDED
   (owner, 2026-08-24): immutable-policy-per-registry.** The consumer profile
   pins the deployment MetaEvidence/policy references; every verification —
   full snapshots (§6 step 3b) and fresh point checks (§8) — proves the
   registry's `metaEvidenceUpdates` counter is still ZERO (slot 9, §5) and
   fails closed on any update. Changing policy means deploying a new registry.
   Implemented in the `intend` CLI with a genuinely-updated-counter rejection
   fixture.
7. ~~OWNER DECISION — semantic file types in trees.~~ **DECIDED (owner,
   2026-08-24): forbidden in V1.** Listing policy v2.1 prohibits symlinks in
   trees and declares executable bits non-semantic (scripts run via explicit
   interpreters); the git-Origin semantic-tree definition drops both. The
   installer profile (rejects symlinks, does not preserve exec bits) is
   thereby the POLICY, not a gap.
