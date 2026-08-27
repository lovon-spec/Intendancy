# Classic Snapshot Benchmarks — Results (RFC 0001 Gate 2)

**Brief**: [`docs/implementation-briefs/0002-snapshot-benchmarks.md`](../implementation-briefs/0002-snapshot-benchmarks.md)
**Crate**: `spikes/snapshot-bench/`
**Date**: 2026-08-23 (revision 3 — remediates Codex review rounds 1 and 2, bus `m-01m0rhn50htncp153c` / `m-01m0rnqt2fpjhve9na`)

## 0. Environment pin (all numbers below were RE-measured under this pin)

| | |
|---|---|
| Hardware | Apple M5 Pro (Virtual), 13 cores, 12 GiB RAM |
| OS | macOS 26.4 (25E246) |
| Toolchain | rustc 1.94.0, cargo 1.94.0 (`--release --locked`, in-tree `Cargo.lock`) |
| Anvil | 1.5.1-stable (b0a9dd9), `--fork-url https://rpc.gnosischain.com --fork-block-number 47881774 --prune-history --transaction-block-keeper 16` |
| Fork block | 47,881,774, hash `0x0d3ad04bbbfdc3403b50444660d54850827b6f7f9dc3e2d4eb2469dbc2bb63d4` |
| Registry | `0x2a25ec70329918d13ec23477716a159d063735d0` (fresh factory deploy per run; the address repeats because each run starts from the same fork state), runtime codehash `0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d` — identical to the Gate 1 canary generation |
| Anchors | 1k: block 47,882,783, hash `0x48e9b2aa…73ec10`, root `0x9e7871ed…` · 10k: block 47,891,783, hash `0x133ef562…ef0184`, root `0xdb17c998…` (the snapshot and profile pin block NUMBER, HASH, and state root — all three compared) |
| Vectors | node v25.8.2, viem 2.47.6 (the frontend's own dependency) |

Exact commands and artifact SHA-256 checksums: §8. The anchor root in fork mode is
the root node of a probe proof (`keccak256(accountProof[0])`) because anvil's
fork-mode headers carry no state root — anchor TRUST remains Gate 1's scope by
design. `--prune-history` is part of the pin: without it anvil was OOM-killed twice
on a memory-pressured host in the first measurement session (a provider-environment
note, not a protocol property; that session's slower wall numbers are superseded by
this rerun).

## 1. Snapshot generation and verification

| Metric | 1,000 items | 10,000 items |
|---|---|---|
| Seed (submit + status choreography, anvil) | 0.77 s | 7.7 s |
| Generate wall total | 0.56 s | 38.7 s |
| — row sweep (sequential `getItemInfo`, one call/item) | 0.13 s | 1.3 s |
| — proof harvest (`eth_getProof`, 250 keys/call → 9 / 81 calls) | 0.39 s | 36.9 s |
| Snapshot raw JSON | 3.05 MB | 31.97 MB |
| Naive per-key proof bytes | 3.74 MB | 46.64 MB |
| Dedup node PAYLOAD bytes | 0.60 MB (6.2×) | 6.08 MB (**7.7×**) |
| Full proof dictionary (payload + 32-B keys + path refs) | 1.08 MB (3.5×) | 11.50 MB (**4.06×**) |
| Unique MPT nodes | 4,441 | 45,235 |
| gzip(JSON) (flate2 default) | 1.28 MB | 13.60 MB |
| Verify: bounded read + JSON parse | 1.8 ms | 22.9 ms |
| Verify: proof verification (§6 steps 2–6) | 9.5 ms | 114.4 ms |
| Verify: descriptor policy screening (§6 step 7 subset) | 0.3 ms | 2.5 ms |
| **Verify: parse + verify + screen total** | **11.7 ms** | **139.9 ms** |
| Verify total from gzip transport form | 18.8 ms | 214.2 ms |
| Verify CPU (user+sys, `/usr/bin/time -l`) | 0.01 s | 0.14 s |
| Verify peak RSS | 15.3 MB | 68.8 MB |
| Slot proofs checked (length + list + status) | 2,001 | 20,001 |
| Status mix (Absent/Registered/RegReq/ClearingReq) | 1/1/997/1 | 1/1/9,997/1 |

Measurement-language corrections from review round 1, now the standing definitions:
totals include parse (parse was and is NOT dominant — 22.9 ms of 139.9 ms at 10k);
proof-call counts are 9 (1k) and 81 (10k); **7.7× is payload-only dedup** while the
honest full-dictionary ratio (adding store keys and per-slot path references) is
**4.06×**; generation is proof-harvest-dominated — now MEASURED per phase (36.9 s
vs 1.3 s at 10k) rather than asserted. No binary-framing sizes are quoted anywhere
in this document because none were measured; framing is CHANGES-REQUIRED (§7.3).

### Verification scope

Profile binding first, then spec §6 steps 2–6, then the offline step-7 subset:

0. **Bounded input at the file boundary** (round 2 finding 1): the CLI reads the
   untrusted snapshot through `read_snapshot_file_bounded`, which sniffs the gzip
   magic and streams via `Read::take` at the applicable cap — at most cap + 1
   bytes are ever read FROM DISK regardless of the on-disk size, BEFORE any
   parse. The logical RETAINED length is bounded per buffer: the on-disk
   buffer holds ≤ the applicable cap + 1 bytes, and gzip input additionally
   materializes a separately capped decoded buffer (≤ decoded cap + 1), so the
   total retained bytes are bounded by the sum of the two caps (heap capacity
   can exceed the logical length by the allocator's growth overhead — `Vec`
   over-allocates; the bound is on bytes retained). All cap arithmetic saturates, so degenerate injected
   limits (zero, tiny, `u64::MAX`) reject or pass cleanly rather than under- or
   overflowing. File-boundary regressions cover oversized raw, oversized gzip,
   zero-cap, one-byte-cap boundary, and `u64::MAX`-cap inputs.
1. **Pinned verifier profile** (review finding 1; round 2 finding 2): `verify`
   requires a `VerifierProfile` — version, chainId, registry address, registry
   runtime codehash, anchor block number, anchor block HASH, anchor state root —
   pinned OUT of band (in production: registry pin + Gate 1 anchor; in the bench:
   the seed manifest). Every identity field, the block hash included, must match
   the snapshot's claims exactly BEFORE proofs are consulted, and after the
   account proof verifies, the PROVEN `codeHash` must equal the pin.
   The attack this closes is a candidate fixture (`eoa-snapshot-25.json`): an
   honest proof that a funded EOA holds an empty `itemList` slot under the
   authentic root, packaged as an empty catalog. It is rejected on the registry
   pin, and — under a deliberately misconfigured profile pinning the EOA address —
   still rejected on the codehash pin (an EOA proves `keccak256("")`).
2. Account proof → proven storageRoot → proven `itemList.length` cross-checked
   against `itemCount` and row count → every contiguous `itemList[i]` slot →
   `keccak256(descriptor) == itemID` per row plus six-column decode → every status
   slot with the 0..=3 bound.
3. Structural policy screening per row (offline-checkable subset of step 7):
   Reserved must be empty, Tree CID must be strictly canonical (multibase `b`,
   base32 lower, no padding, `0x01 0x70 0x12 0x20` + 32-byte digest). Frontmatter
   byte-match and Origin binding need the tree and remain the production CLI's
   scope.

### Status coverage and a bug the review's requirement caught

Review finding 2 (only status 2 had evidence) is remediated by seed choreography:
items 0/1/2 are driven through the real GTCR state machine to Registered (1),
executed-removal Absent (0), and ClearingRequested (3). Exercising the Absent path
immediately caught a real verifier bug: the status slot of a removed item is zero,
and the verifier expected only the MPT **exclusion** form — but anvil's fork-mode
synthetic trie emits an explicit `RLP(0x80)` leaf for slots it has locally zeroed
(a real Ethereum trie deletes on `SSTORE 0`). Both forms prove "value is zero"
against a committed root and neither can prove a nonzero slot, so the verifier now
accepts either, with the fixture covering the explicit-leaf form and the EOA
fixture's empty-trie slot-13 proof covering the true exclusion form end-to-end
(`eoa_snapshot_verifies_under_a_profile_that_pins_the_eoa`).

**Adversarial spot checks** (offline, in-tree candidate fixtures): the snapshot
mutation suite is 24 test functions — 21 rejecting (22 rejection assertions; the
EOA case asserts under two profiles) and 3 positive (pristine all-status verify,
Absent-slot-proves-zero, EOA-under-its-own-profile). Rejections: wrong trusted
root; wrong profile registry/codehash/version/height/BLOCK-HASH/chain; tampered
binding registry and binding block hash; EOA empty catalog (both profile
variants); flipped status claims including Absent↔Registered in both directions;
tampered descriptor; non-contiguous index; missing slot proof; corrupted node
store; mutated slot value; truncated rows; substituted itemID; overclaimed
itemCount.

## 2. Descriptor encoding: independent vectors (review finding 3)

Classic stores opaque bytes, so factory submission alone proves nothing about
cross-implementation compatibility. `fixtures/descriptor-vectors.json` now carries
7 vectors produced OUTSIDE the Rust crate by
`tools/gen-descriptor-vectors.mjs`: 4 through the repo's real
`frontend/src/lib/encoder.ts` (viem `toRlp` over `stringToHex` columns, plus the
frontend's own validation and decode round-trip), 3 edge cases via raw viem
`toRlp` where the frontend validation deliberately refuses the fields (empty
columns, placeholder CIDs, populated Reserved). The Rust codec byte-matches every
vector in both directions and agrees on all itemIDs; the vector set also
partitions correctly under policy screening (realistic passes; Reserved-nonempty
and empty-CID decode fine at the codec layer and are rejected at the policy
layer). Negative codec tests: 5 columns, 7 columns, trailing bytes all fail
`decode_exact`.

## 3. Resource limits (review finding 4)

All bounds live in an injectable `Limits` struct; the limit suite
(`tests/limits.rs`) is 21 test functions — 17 rejecting, 4 positive — and every
enforcement BRANCH now has its own isolated rejection (round 2 finding 3 noted
that itemCount shadowed row-count, account-node shadowed storage-node, and
account-path shadowed storage-path; the four shadowed branches — row count, slot
count, storage-node size, storage-path length — are now hit directly by
constructing inputs that pass the earlier checks). Covered: itemCount cap; row
cap; slot-proof-count cap; node count cap; per-node size cap separately for
ACCOUNT and STORAGE nodes; aggregate node-store bytes; proof path length
separately for account and storage paths; a REAL bounded gzip reader (compressed
cap; decoded cap enforced via `Take` — a gzip bomb of 8 MiB zeros is stopped at
the decoded cap); the raw-input decoded cap; and the FILE-boundary reader
(oversized raw and gzip files rejected after at most cap + 1 read bytes, without
materializing the input — round 2 finding 1).

**The limit VALUES are CHANGES-REQUIRED and deferred** to the binary framing
decision (§7.3): at measured density (~3.2 KB/item raw JSON, ~1.15 KB/item full
dictionary), a 256 MiB decoded-JSON cap holds only ≈80k items, so the previous
draft's "1M items within 256 MiB" was incoherent for the JSON transport and is
withdrawn. The current working values (64 MiB compressed / 256 MiB decoded /
16 KiB node / 8M nodes / 192 MiB store / 1M items / 66-node paths) bound THIS
prototype; 1M items is a verifier sanity cap, not a transport claim. Final numbers
freeze together with the framing.

## 4. Install path (CAR/UnixFS) — **NO-GO for production** (review finding 5)

Per brief 0002 §Goal/3, the honest result for this phase is a precise NO-GO.

**What the prototype now does** (the review-round-1-listed defects plus round 2's
rehash gap are addressed, so the retained timings describe defensible mechanics —
this is NOT a claim that all defects are fixed; see the residual list below): the
expected Tree CID is an EXTERNAL input (`install(expected, …)`; in production it
comes from the verified descriptor — the CAR's own root claim is never the
authority); `preflight`/`install` REHASH every block on first visit, so the
CID→bytes map's keys are never trusted even when the map did not come from
`read_car` (round 2 finding 4; wrong-bytes-under-valid-key is tested for a leaf
and for the root node); full preflight validation with NO filesystem writes on any
failure path; staging into a fresh sibling directory published by ONE atomic
rename; the destination is refused if ANYTHING exists there (file, dir, symlink —
checked with `symlink_metadata`, so dangling symlinks are caught) after
canonicalizing the parent; failed installs remove their staging directory (tests
assert no litter and no destination); legitimate shared blocks (same CID referenced
by several links, block shipped once) install correctly for files and
subdirectories; the file/dir-count bounds are enforced per plan entry (effective
within a single directory — the old per-directory check was not); the 2 MiB
listing-policy cap is enforced over MATERIALIZED bytes (one 1 MiB block shared by
three links is rejected at 3 MiB); the lockfile records the tree CID, root block
sha256, block/byte totals, and per-file sha256, and records the ABSENT registry
identity as `null` instead of placeholder strings (the identity schema now carries
the anchor block hash for when the production CLI wires it in).

**Why it is still NO-GO**: the parser supports only the profile
{UnixFS Directory nodes (non-HAMT) + single-block raw leaves}. Ordinary chunked
dag-pb File nodes (multi-block files with blocksizes), HAMT-sharded directories,
and any layout produced by other builders' chunking choices are rejected, and
there is NO interop evidence — kubo is not installed on this host, so no
kubo-produced CAR was parsed and no kubo CID was reproduced. A production
installer must either (a) adopt a maintained CAR/dag-pb/UnixFS read stack (survey
candidates: `rs-car`/`car-utils` for CAR framing plus `ipld-core` +
`ipld-dagpb` for node decoding, with our policy/sanitization layer on top — exact
pin selection is follow-up work with its own review), or (b) freeze a bounded
"UnixFS-basic" producer/consumer profile (CIDv1 sha2-256, raw leaves ≤ 256 KiB,
balanced single-level File chunking, plain Directory nodes) and prove it against
kubo-generated fixtures. **Acceptance is gated on kubo interop vectors either
way.**

Prototype timings (non-production evidence, self-built trees; the overhead
denominator is the full materialized tree INCLUDING the generated `SKILL.md`):

| Metric | ~50 KB tree (7 files) | ~1 MiB tree (5 files) |
|---|---|---|
| Build + CAR encode | 0.3 ms | 3.4 ms |
| Read + preflight (incl. rehash) + staged install + publish | 0.9 ms | 6.1 ms |
| CAR overhead vs materialized bytes | +1.72 % | +0.064 % |

**Install-path adversarial checks** (17 test functions: 13 rejecting, 3 positive
installs, 1 mixed CID-text round-trip): tampered block payload at read; wrong
bytes under a valid CID key at preflight (leaf and root); root substitution
refused before any write; unreachable padding block; missing child block with
clean failure; existing destination dir; destination symlink (target not written
through); DANGLING destination symlink; shared raw block installs under both
names; shared subdirectory installs under both names; file-count bound effective
within one directory (10,001 links, one shared block); materialized-byte policy
cap (3 × shared 1 MiB); path traversal names (`..`, `.`, `a/b`, `a\b`, empty,
NUL); duplicate entry names; disallowed codec; uppercase CID text; byte-identical
round-trip.

**Known residuals** (bench scope — DOCUMENTED, deliberately not built out here;
all are production blockers on top of the parser-profile/interop gap above):
the publish step is check-then-rename, so a same-UID racer between the destination
check and the rename can still make the rename fail or, for an empty directory
created in the window, be replaced; bytewise-distinct entry names can COALESCE on
case-insensitive/normalization-insensitive filesystems (APFS default), so two
policy-valid names may collide at extraction; content publish and lockfile write
are separate steps, not one transaction; the staging directory is not created
0700/private and same-UID path races around it remain; the varint reader accepts
noncanonical and does not reject all overflowing encodings, and some length
additions are unchecked arithmetic; duplicate protobuf fields within a PBNode/
PBLink are not rejected; full UnixFS layouts (chunked File nodes, HAMT) are
unsupported; kubo interop is undemonstrated.

## 5. Gates

`cargo test --locked`: **69/69** — 4 schema units (2 rejecting / 2 positive),
3 vector suites, 24 snapshot mutations/bindings (21 rejecting / 3 positive),
21 limits (17 rejecting / 4 positive), 17 CAR (13 rejecting / 3 positive
installs / 1 mixed). `cargo clippy --all-targets --locked -- -D warnings`: clean.
`cargo fmt --check`: clean. Fixtures regenerate with the commands in the README;
candidate-fixture checksums in §8.

## 6. Freeze recommendations (verified-snapshot-spec §11)

1. **Storage layout + descriptor encoding — FREEZE-READY.** Slot constants (13/14)
   re-confirmed against freshly factory-deployed registries at both scales at the
   pinned fork block; deployed codehash matches the Gate 1 canary generation; the
   six-column RLP now byte-matches INDEPENDENT vectors from the frontend's viem
   encoder in both directions (this was the missing evidence — self-round-trip
   alone was not it).
2. **Deduplicated node store — PRINCIPLE SUPPORTED, framing not frozen.** Payload
   dedup is 7.7× at 10k; the full dictionary including keys and path references is
   4.06× — the honest number for transport budgeting. Freeze the exact reference
   encoding together with recommendation 3.
3. **JSON transport — CHANGES-REQUIRED** (unchanged). Keep JSON as the debug
   encoding; adopt length-prefixed binary framing (or CBOR) as the canonical wire
   format with REQUIRED compressed content-encoding. Measured sizes to publish
   today: 1.28 MB (1k) / 13.60 MB (10k) gzip-JSON. No binary sizes are claimed;
   measure them in the framing spike.
4. **Resource limits — CHANGES-REQUIRED, values deferred to the framing spike.**
   The MECHANISMS (injectable bounds incl. bounded decompression, account-node and
   path caps) are implemented and adversarially tested and should carry into the
   spec as normative requirements; the VALUES freeze with the framing. The JSON
   transport tops out ≈80k items at 256 MiB decoded — a further argument for 3.
5. **Provider generation — no spec change.** Proof-harvest-dominated (36.9 s of
   38.7 s at 10k against anvil; 81 calls at 250 keys/call), with the row sweep at
   1.3 s. Production nodes and parallel chunking will improve constants.
6. **Verifier profile binding — ADD TO SPEC (from review findings R1-1, R2-2).**
   §6 must require the pinned profile (version, chain, registry, runtime codehash,
   anchor block number AND HASH, state root) as verification step 1 and the
   proven-codehash equality as step 2b. Without it, complete enumeration is
   enumeration of the WRONG account. The spec's anchor is the finalized
   block/hash pair, so the snapshot envelope and the lockfile identity both carry
   the block hash.
7. **Install path — NO-GO as specified in §4.** Approach semantics (external CID
   authority, preflight-then-atomic-publish, materialized-byte policy cap, shared
   blocks) are the right consumer contract and should inform the spec text, but
   production acceptance is gated on a maintained/bounded parser decision plus
   kubo interop vectors.

## 7. Remediation log (review round 1 → this revision)

**Round 1** (`m-01m0rhn50htncp153c`), all seven findings conceded and addressed:
(1) profile binding implemented + rejection tests + EOA fixture; (2) all four
statuses seeded, zero-slot exclusion AND explicit-zero-leaf forms covered — and
the requirement caught a real verifier bug, fixed; (3) independent frontend/viem
vectors + policy screening with negative tests; (4) injectable limits incl.
bounded gzip reader, values marked deferred, incoherent 1M/256MiB claim
withdrawn; (5) install path reclassified NO-GO with the listed prototype defects
addressed and interop gated on kubo vectors; (6) measurement language corrected
as itemized in §1; (7) full rerun under the §0 pin with per-phase timing, CPU,
RSS, commands, and checksums.

**Round 2** (`m-01m0rnqt2fpjhve9na`), all five findings conceded and addressed:
(1) HIGH — byte caps now enforced at the FILE boundary before over-cap
accumulation
(`read_snapshot_file_bounded`, `Read::take`-streamed, at most cap + 1 bytes
read / logically retained; file-boundary
regressions added); (2) the anchor block HASH is now part of the snapshot
envelope, the verifier profile (compared exactly), and the lockfile identity,
with wrong-hash rejection tests, and fixtures regenerated; (3) the four
shadowed bound branches (row count, slot count, storage-node size, storage-path
length) got isolated rejection tests and the coverage claim was made precise
(§3); (4) preflight/install rehash every block on first visit with a
wrong-bytes-under-valid-key test, and the remaining prototype gaps are documented
as residuals rather than fixed (§4); (5) stale exclusion comments corrected in
`chain.rs`/`mutations.rs`, test counts stated precisely, CAR overhead recomputed
against the full materialized denominator (1.72 % / 0.064 %), and fixture wording
changed to "candidate" since Gate 2 files remain untracked pending an authorized
commit.

**Round 2 micro-review** (`m-01m0rpymp8y0jv87rz`): the file-boundary reader's
`cap + 1 - got` was unchecked — with an injected zero cap and a ≥2-byte file it
underflowed (debug panic; release wrap defeating the read bound) — and two more
cap sites (`decoded cap + 1`, `1 + 2 × max_items`) could overflow at `u64::MAX`.
All cap arithmetic is now saturating with an early over-cap rejection of the
sniffed bytes, covered by zero-cap, one-byte-cap-boundary, and extreme-cap
regressions. The "cap + 1 allocation" wording was corrected to per-buffer RETAINED-LENGTH
bounds — heap capacity may exceed the logical length by allocator growth
(§ Verification scope, step 0) — stale exclusion-proof comments were fixed in the
test/CLI text, and the two remaining stale prior-run metrics in this document
were updated to the current rerun's numbers.

## 8. Reproduction

```bash
# Terminal 1 — pinned fork (the flags are part of the pin):
anvil --fork-url https://rpc.gnosischain.com --fork-block-number 47881774 \
      --port 8547 --prune-history --transaction-block-keeper 16

# Terminal 2 — from spikes/snapshot-bench (release, locked):
cargo build --release --locked
B=./target/release/snapshot-bench
/usr/bin/time -l $B seed     --rpc-url http://127.0.0.1:8547 --items 1000  --out /tmp/m1k.json
/usr/bin/time -l $B generate --manifest /tmp/m1k.json  --out /tmp/s1k.json
gzip -kf /tmp/s1k.json
/usr/bin/time -l $B verify   --manifest /tmp/m1k.json  --snapshot /tmp/s1k.json
$B verify --manifest /tmp/m1k.json --snapshot /tmp/s1k.json.gz
# restart anvil (fresh fork), then identically with --items 10000 → m10k/s10k
$B car-bench --files 6 --file-bytes 8192   --work /tmp/car-small
$B car-bench --files 4 --file-bytes 262144 --work /tmp/car-large
```

Measured-artifact SHA-256 (large snapshots are reported, not committed, per brief):

```text
fee0528c6e0b051afe17e303a5711319274c762853e88f5e8e771a6c2bc90660  m1k.json
21b947b2528197e73825e5860fa102295abd13a9473e73879a6224500fb77c9c  s1k.json
2cadd39f080aaa993c77888c4e1f5add0547757f92f02aeb7f133dfd62089f14  m10k.json
540972565c592ffd59e3ee6b9851b84db7034dc396089f6852b80d6fbade8b29  s10k.json
```

Candidate-fixture SHA-256 (in-tree under `spikes/snapshot-bench/fixtures/`,
repo-UNTRACKED pending an authorized commit; regeneration commands in the crate
README):

```text
d09eeaf6aefe3154335e50bdb034d996948e0a75d00bd19699efbeeb1fbf1762  fixtures/manifest-25.json
7f6ea1eb285e09b6d65b1327d68895235d5be74eb07f06840cc7f889e5e05a30  fixtures/snapshot-25.json
101e37af576144d5dbbf4b56fbf187e4abf786a7c95ba7de3b91d6cc997e88a8  fixtures/eoa-snapshot-25.json
8a0aa89c01472291b68d57a299ebb015138765d3862b0bfc8699047147e3a3f0  fixtures/descriptor-vectors.json
```
