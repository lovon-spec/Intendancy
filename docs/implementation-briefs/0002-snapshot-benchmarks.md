# Implementation Brief 0002: Classic Snapshot Benchmarks (RFC 0001 Gate 2)

**Status:** Self-authored by the implementer per owner direction; open to reviewer amendment
**Implementer:** Claude
**Reviewer:** Codex
**Scope owner:** `spikes/snapshot-bench/**` and `docs/spikes/snapshot-bench-results.md` only

## Why this is next

[RFC 0001 §7](../rfcs/0001-intendhub-v1-decision.md) gates the snapshot wire-format and
implementation freeze on two validations. Gate 1 (the Gnosis consensus anchor) is signed
off. This brief covers Gate 2:

> Benchmark provider generation and consumer verification for complete 1,000- and
> 10,000-item Classic snapshots, including proof deduplication, transfer size, CPU,
> memory, resource limits, CAR verification, path sanitization, and exact lockfile
> installation.

The [draft snapshot spec](../verified-snapshot-spec.md) §4 marks its JSON envelope and
hash-keyed trie-node store as a **provisional transport**; these measurements decide the
final framing, compression, and limits.

## Goal

Produce a runnable, non-production Rust spike with three measured phases:

1. **Provider generation.** Against a local Anvil fork of Gnosis, deploy fresh Classic
   registries through the REAL `GTCRFactory` (`0x794Cee5a…FE039`) and seed them with
   1,000 and 10,000 synthetic entries encoded with the **V1 six-column descriptor
   schema** (Name, Description, Tree CID, Runtimes, Origin, Reserved-empty) — the bench
   dogfoods the production RLP encoding and canonical-CID column format. Harvest
   `eth_getProof` for the `itemList` length slot, every `itemList[i]` slot, and every
   item status slot at one pinned anchor block; emit the spec-§4 snapshot (rows +
   deduplicated node store). Measure: generation wall time, raw JSON bytes, deduplicated
   bytes vs naive per-key proofs, and gzip bytes.
2. **Consumer verification.** Verify the complete snapshot per spec §6 steps 2–6
   (account proof → storageRoot → proven length → contiguous enumeration → descriptor
   re-hash → status sweep) against the anchor stateRoot, with explicit input-size
   bounds. Measure wall time, CPU time, and peak memory at both scales; verification
   MUST reject any single mutated row/proof (spot adversarial checks, not the full
   Gate-1 matrix).
3. **Install path.** Verify a complete CAR/UnixFS skill tree against its Tree CID
   block-by-block (hashes, link structure, DAG completeness), reject unsafe paths
   (absolute, `..`, symlink escapes), install the exact bytes, and write a lockfile
   recording chain, registry, item ID, Tree CID, and anchor. Measure at a small
   (~50 KB) and a large (~1 MiB) tree. If no suitably maintained Rust CAR/UnixFS
   verification primitive exists, a precise `NO-GO` naming the missing primitive and
   the smallest bounded implementation is a valid result for this phase.

## Method constraints

- Anchor trust is OUT of scope here (Gate 1 owns it): the anchor stateRoot is taken
  from the local Anvil chain and treated as given. No light client, no header quorum.
- The snapshot bench crate must NOT depend on `helios-consensus-core` (no BLS); its
  proof verification mirrors the same `alloy-trie` pattern Gate 1 uses.
- Seeding runs against `anvil --fork-url` so the real factory bytecode deploys the real
  registry generation; the storage layout under proof is therefore the deployed one,
  not a simulation. Pin and record the fork block and the deployed registry addresses
  and codehash.
- Exact released crate versions or pinned revisions; committed `Cargo.lock`; offline
  determinism is NOT required (this spike measures a live local chain), but every
  number in the results document must name its exact reproduction command.
- Only the scope-owner paths may be touched. No commit/stage/remote/push.

## Deliverables

1. `spikes/snapshot-bench/` — isolated crate (own `[workspace]`), subcommands:
   `seed`, `generate`, `verify`, `install`; README with exact commands.
2. Checked-in small sample artifacts where they keep the repo light (the 10k snapshot
   itself is measured, reported, and NOT committed).
3. `docs/spikes/snapshot-bench-results.md`: the measurement tables (1k/10k), the
   dedup-vs-naive and gzip ratios, verify CPU/mem, install-path results, resource-limit
   findings, and a recommendation for the snapshot spec §11 open items (final framing,
   compression, size expectations, limits) — `FREEZE-READY` or `CHANGES-REQUIRED` per
   spec section.

## Acceptance gate

1. Both scales generate and fully verify end to end; mutated-input spot checks fail closed.
2. Every reported number carries its reproduction command and pinned environment.
3. The install phase verifies a real multi-file tree byte-for-byte against its CID with
   sanitized extraction and an exact lockfile — or delivers the specified `NO-GO`.
4. The results document gives the spec-freeze recommendation per open item.

## Non-goals

The consensus anchor, light client, header quorum; publishing snapshots anywhere; the
`intend` product CLI; registry policy/MetaEvidence changes; production deployment.
