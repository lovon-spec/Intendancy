# Intendhub repository guidance

## Validation

Run the checks for every component touched by a change:

- `cd contracts && forge fmt --check && forge test`
- `cd frontend && npm run lint && npm run build`
- In each existing Rust crate (`cli`, `spikes/gnosis-anchor`, and `spikes/snapshot-bench`): `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --locked`
- When either listing-policy copy changes: `cmp docs/listing-policy.md frontend/public/listing-policy.md`

The full CLI smoke test requires both `cli/` and `spikes/snapshot-bench/`; do not treat its absence from an independent PR branch as a failure. Benchmark regeneration, live probes, and Kubo-vector regeneration are evidence workflows, not routine review gates.

## Code Review Rules

- Treat snapshots, CDN/indexer and RPC responses, EIP-1186 proofs, CAR/gateway data, and pre-existing local state as attacker-controlled. Trust roots must come from the locally authenticated deployment profile or embedded consensus path. Flag provider-controlled chain/deployment/codehash/storage-layout/policy/finality inputs, cross-deployment replay, rollback, or stale state presented as current.
- A catalog is complete and trustless only when one authenticated finalized anchor binds block number, block hash, and state root, and that state proves the pinned registry/codehash, `itemCount`, every contiguous `itemList[i]`, descriptor-to-item-ID hashes, statuses, and immutable policy (`metaEvidenceUpdates == 0`). Install, audit, and enable also need a fresh exact-item status/policy proof. Flag point or sampled proofs, header/RPC quorum, modified Classic GTCR bytecode, or unpinned factory/arbitrator/deployment parameters described as equivalent.
- Content installation must derive authority from the verified descriptor CID, rehash the complete reachable DAG, reject extra blocks, symlinks, unsafe names and normalization/case collisions, enforce limits before materialization, and publish with serialized no-replace, crash-consistent durability and sticky quarantine/revocation. Security-contract changes to encodings, slots, proofs, deployment pins, limits, or availability claims must update implementation, specs, fixtures, and reproducible evidence together; the two listing-policy copies must remain byte-identical, and integrity/current retrievability must never be called proof of review-period availability.
