> **Supersession notice:** This is historical review material. [RFC 0001: Intendancy V1 Owner Decision](./0001-intendancy-v1-decision.md) is canonical; any conflicting conclusion here is superseded.

# Review of RFC 0001 — Claude

**Reviewing**: `0001-intend-architecture-review.md`
**Mode**: architecture review, read-only; no implementation or normative files touched. Two proposals in §§8–9 amend positions I previously drafted; they are proposals for the owner, not edits.

---

## 1. Verdict

**Sound abstraction, over-generalized as a V1 plan.** The layer the RFC identifies is real: applications consume trusted JSON projections of chain state everywhere (token lists, tags, package indexes), and nobody verifies source, completeness, or transformation. "Light-client-backed verifiable materializer" is the correct technical name; keep it and drop "reverse oracle" (oracles carry facts *onto* chains; this is a light client with projections, and the metaphor will mislead).

But the plan's gravity pulls toward building the generic framework before the wedge has a single production consumer. The failure mode has a name — inner-platform effect — and the RFC's own §13 sequence flirts with it (foundation RFC first; T2CR and Token Lists adapters scheduled before the package manager has users). The abstraction should be **extracted from a working Intendancy pipeline, not specified ahead of it**. Concretely: V1 ships one binary; the `intend` library boundary is drawn where the code naturally cleaves (consensus client, MPT/proof verification, CAR/DAG verification — all genuinely generic); the adapter *interface* and capability *taxonomy* freeze only after the second real adapter exists. Writing the foundation RFC as a document is cheap and fine; implementing to it before the wedge ships is the mistake to refuse.

## 2. Strongest objections

1. **§10's availability paragraph swings back into the bait trap.** "Material public retrievability throughout registration/dispute/appeal" as an adjudicated requirement is, as stated, exactly the construction the project owner's griefing analysis killed: availability is submitter-toggleable, retrievability history is unprovable testimony, and a late reveal either cures (challenger harvested) or the rule collapses into juror liveness-divination. The RFC is right that the drafted Principle B has a real cost (see §8 — I concede the "listed-but-untrusted" pollution objection), but the fix is a timing-anchored rule, not a return to adjudicated liveness. §8 proposes the synthesis.
2. **"Locally pinned" relocates trust; it does not remove it.** Adapters, projections, checkpoints, and the verifier itself arrive via the release channel — the true TCB is **release signing + reproducible builds**, and the RFC never names it. An adapter "pinned locally" that was fetched from the same ecosystem that serves the data is circular at one remove. Mitigations: adapters as *declarative data* (addresses, codehashes, slot rules, enum maps — no code) wherever possible; projections Turing-incomplete in V1; signed releases with independent checkpoint sources; conformance vectors in-repo.
3. **The schema objection is correct and the RFC under-solves it.** Bond Reference is the wrong use of column 6, and without identity/version/supersession, `upgrade` cannot be safe. But the RFC stops at naming the gap (Q8) while fearing the naming-authority trap. §9 proposes a solution with no namespace at all.

## 3. Overclaim audit

§4's non-claims are honest and should survive verbatim. Three watch-points: (a) "deterministic transformation" must not imply *faithful* transformation for enriched outputs — any field not derived from authenticated inputs needs a per-field provenance class (`authenticated` / `derived` / `enriched-trusted`); (b) `historical-complete` conflates two facts — completeness of the *ever-admitted universe* and *currency of statuses at the anchor* — the grade should say "complete enumeration, statuses as-of-anchor"; (c) the taxonomy lacks a **freshness axis**: anchor mode covers header authenticity, capability covers completeness, but staleness policy (finalized-only, max anchor age, rollback refusal) is a third dimension the verifier computes and should be named alongside the other two.

## 4. Component boundary (revised)

Keep the three-way *conceptual* split (`intend` / Intendancy / Intendant). Reject the three-way *implementation* split for V1. One repository, one binary, three internal layers with the future seam lines respected:

- **Generic now** (because analogues already exist and the code is inherently reusable): consensus light client, EIP-1186/MPT verification, CAR/UnixFS verification, bundle envelope parsing.
- **Concrete now, generalize later**: the Classic-GTCR adapter (hardcoded, with its declarative core factored as data), the skills-catalog projection (the only projection), install/audit/lockfile logic.
- **Not now**: adapter plugin interface, projection manifest machinery, capability taxonomy as public API, stateless-EVM primitive, SQLite/API outputs, second and third adapters.

## 5. Smallest coherent V1

1. Gnosis light-client spike (agreed: highest risk, do first — strategy in Q7).
2. Snapshot verification per the drafted Verified Snapshot spec: anchor → account proof → count → enumeration → descriptor rehash → status sweep.
3. `intend update` (verify snapshot), `intend install` (fresh finalized point check, CAR-verified tree, path sanitization, lockfile), `intend audit` (installed CIDs vs fresh status + tombstones).
4. Ingestion/possession evidence flow per §8's rule (submitter auto-posts tree evidence; archivers post receipts).
5. Header-quorum as default mode; RPC-quorum fallback; light-client strict mode when the spike lands.

**No `upgrade`** until the §9 lineage schema is frozen — the RFC is right that it blocks. No projections beyond the catalog. No second adapter.

## 6. Deferrals

T2CR adapter (good validation, wrong time — after Intendant integration works); LGTCR `sound-only` adapter and Token Lists projection; stateless view execution (see Q5 for the convergence note); PGTCR bonded tier (schema no longer reserves it — §9); current-set/historyHead upstream work (file the Curate V2 issues now, build against them never for V1); generic projection manifests (Q11).

## 7. Freeze list before policy/MetaEvidence/encoder/tests are amended

1. **Availability rule** — Principle B as drafted vs §8's timing-anchored rule. Owner decision; everything juror-facing depends on it.
2. **Column 6** — Bond Reference out, `Predecessor` in (§9), plus the release-manifest-in-tree requirement.
3. Slot constants and encoder byte-compatibility verified against deployed bytecode; codehash pinned.
4. Agent Skills spec commit pin.
5. Anchor-mode defaults, finality requirement, max snapshot age numbers.
6. Naming (`intend`) — non-blocking, decide any time.

---

## 8. Proposal: the availability synthesis (answers Q10)

The drafted Principle B (never adjudicated; dark entries list with degraded labels) has the cost the RFC implies: it mints `Registered` statuses that conforming installers must then distrust — semantic pollution of the registry's core signal. The RFC's counter-rule (adjudicated retrievability) resurrects the bait-grief. Both are wrong in opposite directions. The synthesis uses the one availability fact that *is* provable — **on-chain evidence timestamps** — and one asymmetry: **who** produces the bytes.

**Rule (proposed policy amendment):**

1. At submission, the submitter SHOULD post the complete skill tree (CAR or equivalent) as ERC-1497 evidence; submission tooling does this automatically. Independent archivers fetch and post possession receipts as already specified.
2. "Content not publicly retrievable" is a valid **registration-challenge** ground, adjudicated purely on the evidence record's timestamps:
   - If **independent possession evidence predates the challenge** (any party other than the submitter: archiver receipt, evidence post, juror-verifiable copy), the challenge **fails**. The challenger could have checked the record; deposits flow accordingly.
   - If **no independent possession evidence predates the challenge**, the challenge **succeeds** — and, critically, **late production by the submitter does not cure**. The entry is rejected; the submitter may resubmit with the content now public (new deposit, new window).
3. Post-listing, permanent availability is not required and not adjudicated (unchanged); during any dispute, produced hash-matching bytes remain the artifact regardless of source (unchanged Principle C).

**Why this closes both failure modes:** The bait-griefer hides good content, provokes a challenge, reveals — and *loses their deposit*, because self-production after the challenge cures nothing. Baiting now funds challengers. The dark submission gets cheaply rejected instead of listing-with-asterisk, restoring `Registered` to full meaning. The honest submitter is safe by default: tooling posts the tree at submission, archivers ingest within minutes, and any later "it was dark" challenge auto-fails on the prior timestamps. Jurors never weigh fetch testimony — both branches decide on chain-timestamped facts, and possession claims remain probe-checkable. The narrow residual: an honest submitter who bypasses tooling, stays un-archived for days, and draws a challenge loses a deposit despite good faith — a small, self-inflicted, resubmittable edge that prices in exactly the behavior the registry wants to discourage.

Receipts are thereby upgraded from informational labels to **defeating evidence with consensus timestamps**, while staying non-gating and roster-free in adjudication (any independent party's prior evidence counts).

## 9. Proposal: lineage without a namespace (answers Q8)

Identity must not be a name (squat market, transfer disputes, naming authority) and must not be a mutable publisher namespace. Make it a **hash chain rooted at the genesis item**:

- **Lineage ID** = the itemID of a package's first registered version. Not a name — a content-derived identifier nobody can squat.
- **Column 6 becomes `Predecessor`**: the itemID of the version this release supersedes; empty for a genesis release.
- The skill tree MUST contain a **signed release manifest** binding `(predecessor itemID, this Tree CID, monotonic release counter)` under a **lineage key declared in the genesis tree** (rotatable by a statement signed with the prior key, carried in a successor tree).
- **Policy criterion (objective)**: a descriptor with a non-empty Predecessor is valid only if its manifest signature verifies against the lineage's current key and its counter increases. A false lineage claim is rejectable/removable on signature failure alone — juror-checkable, no testimony.
- **`upgrade` semantics**: follow Predecessor edges forward within one lineage; require the target `Registered` plus a fresh finalized check; refuse counter rollback. Display names stay human-facing and are policed only by the existing impersonation criterion.
- Key compromise degrades, not breaks: rotation via the chain; worst case, tombstone the lineage and start a new genesis (the revocation list marks the break publicly). The speculative bond tier, if it ever ships, references lineages from the release manifest — no schema surface consumed.

This gives safe automatic upgrade, rollback protection, and supersession with zero naming authority — and it is a smaller trust surface than Bond Reference was, because every claim it introduces is signature-verifiable.

---

## 10. Answers to the remaining §12 questions

**Q1 — coherent primitive or adapter-hidden client collection?** Both, in sequence. Today it is two contract-specific clients sharing real generic substrate (light client, MPT, CAR). It becomes a primitive the day the third adapter needs no core changes. Build for that day; don't declare it early.

**Q2 — correct abstraction/claim?** "Verifiable materializer" yes. Public claim: *"verifies that served artifacts are faithful projections of finalized on-chain state — complete where the source contract makes completeness provable — and that content-addressed artifacts match their commitments."* Non-claims: keep §4, add "does not verify enrichment fields" per the provenance classes.

**Q3 — anchor × capability separation?** Correct and well-named; add the freshness axis and the `historical-complete` wording fix (§3 above).

**Q4 — hidden TCB?** Release signing and build reproducibility (the big one); the crypto plumbing libraries (BLS/SSZ/MPT/RLP); CID/UnixFS decoders (parser attack surface); checkpoint distribution's social layer; package-resolution logic once lineage exists; lockfile/cache integrity on the local machine.

**Q5 — storage proofs vs stateless view execution?** V1: declared storage proofs only — smallest audited surface, and the bulk enumeration sweep is exactly what they're efficient at. But note the convergence: if the light-client spike lands on a Helios-style stack, verified `eth_call` (witnesses + local EVM against the authenticated root) comes nearly free and would eventually let adapters shrink to "call these getters" declarations, eliminating hand-derived slot math. Adopt it *then*, for point checks first; never build a bespoke EVM path for V1. It still cannot manufacture enumerability — the RFC's own caveat stands.

**Q6 — bundle transport?** Hash-keyed deduplicated node store + EIP-1186 account proof is right for V1 (matches the drafted snapshot spec; measured 7× dedup). JSON now, CBOR framing after the 1k/10k/100k benchmarks. Watch the stateless-Ethereum witness-format work as a future standard to align with; do not invent one.

**Q7 — Gnosis light-client strategy?** Three-track: (a) spike = Helios fork with the Gnosis preset (16 slots/epoch, 512-epoch periods, fork digests, checkpoint bootstrap) — config-and-testing heavy, not research; upstream it. (b) Meanwhile, run Lodestar/Nimbus server-side to cross-check bundles in CI and at the archiver — strict verification exists in the pipeline even before it's embeddable. (c) Ship header-quorum as the client default until (a) lands. Ship a weak-subjectivity checkpoint in every signed release; allow operator-supplied checkpoint sources; cache verified state forward.

**Q9 — uniform quarantine vs trusted emergency requesters?** Uniform fail-closed, split by operation: **new installs** hard-fail on anything but exactly-`Registered`; **already-installed** content gets `audit` warnings on `ClearingRequested` and hard-fails only on executed removal. This bounds the removal-request DoS (an attacker burns a removal deposit to block *new* installs for one challenge period, and loses it when challenged) without granting any requester differential protocol power. The guardian roster stays what it is: UI urgency, zero proof surface.

**Q11 — projections replacing trusted exporters?** Only for lossless projections of authenticated inputs. Enriched outputs (logos, off-registry decimals, publisher timestamps) can be *labeled*, never *proven* — per-field provenance classes, and the honest marketing sentence is "verifies the registry-derived fields; enrichment remains attested." Defer all of it past V1.

**Q12 — cuts?** As §§5–6: cut T2CR, LGTCR adapter, Token Lists, projection manifests, stateless EVM, taxonomy-as-API, `upgrade`-before-lineage, "reverse oracle" branding. The wedge is Intendant installing skills it can prove; everything that doesn't shorten the path to that is V2.

**Q13 — document split?** Two RFCs are fine, with an extraction discipline: write the Intendancy profile RFC first (it has a customer), and let the foundation RFC contain only what the profile actually consumed, plus the frozen naming of the three verification dimensions. A foundation RFC written ahead of its only consumer will speculate; one extracted from it will be true.

---

## 11. Note on §10's characterization of the drafted policy

The drafted policy v2.0 and snapshot spec encoded the design as agreed at drafting time; the RFC's "not the accepted pragmatic direction" reopens a settled point rather than reporting one — which is legitimate (that is what RFCs are for), but the decision belongs to the project owner, and §8 above is my best synthesis of both positions rather than a defense of my draft: it concedes the semantic-pollution objection against Principle B, keeps the owner's griefing theorem satisfied, and gives jurors only chain-timestamped facts. If adopted, the policy amendment touches Principle B, the receipt section (evidence weight, not just labels), and the removal-criteria wording; the snapshot spec's receipt record gains a "defeats-availability-challenges-after" timestamp semantic; nothing else in either document moves.
