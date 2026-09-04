<p align="center">
  <img src="assets/intendancy-mark.svg" width="112" alt="Intendancy mark" />
</p>

<h1 align="center">Intendancy</h1>

<p align="center"><strong>A verifiable, policy-governed registry for agent skills.</strong></p>

Intendancy gives Intendant and other agent runtimes a package catalog whose membership and artifact integrity can be checked without trusting a marketplace backend, CDN, IPFS gateway, indexer, or execution RPC.

The name describes the product boundary: an **intendancy** is the office or domain of an intendant. Here it is the governed capability domain from which an Intendant—or another compatible runtime—can verify and install skills.

## Product family

- **`intend`** is the verification and package-management machinery: chain anchoring, state proofs, CID/CAR verification, safe installation, lockfiles, and auditing.
- **Intendancy** is the first registry profile: agent skills curated by an unmodified Classic Kleros `GeneralizedTCR` on Gnosis Chain.
- **Intendant** is the first consuming runtime.
- **Intendment** is a separate, related settlement protocol; it is not part of the Intendancy runtime path.

## V1

V1 curates directories containing a standards-compliant `SKILL.md`. A client can:

1. authenticate a complete registry snapshot against finalized chain state;
2. verify each descriptor, status, and exact Tree CID;
3. retrieve the corresponding UnixFS DAG from any provider;
4. install exact, sanitized bytes and write a lockfile; and
5. audit installed skills against fresh registry status.

The registry remains optimistic: an unchallenged request may be registered without juror review, while challenged requests are decided under the immutable listing policy. Verification proves what the registry says and which bytes it names; it does not prove that jurors ruled wisely or that a skill is harmless beyond the policy signal.

## Architecture

```text
untrusted CDN / indexer / mirror
  -> snapshot rows + EIP-1186/MPT proof nodes
  -> finalized execution stateRoot authenticated by the client
  -> pinned registry codehash, storage layout, descriptor, and policy
  -> complete item enumeration and status verification
  -> fresh exact-item status check
  -> CAR/UnixFS retrieval from any provider
  -> complete DAG and path verification against Tree CID
  -> sanitized exact install + lockfile
```

Strict mode targets an embedded Gnosis proof-of-stake light client. Header-quorum and RPC-quorum modes are explicitly degraded and must report their trust grade rather than silently falling back.

## Repository layout

- `cli/` — the `intend` verifier and package-manager CLI.
- `contracts/` — deployment tooling and contract-level tests for the stock Classic registry.
- `frontend/` — the registry browser and request interface.
- `docs/` — architecture, policy, specifications, RFCs, and validation evidence.
- `meta-evidence/` — registration and clearing MetaEvidence documents.
- `spikes/` — bounded consensus-anchor and snapshot-performance validations.

## Status

The V1 architecture is accepted, but proof transport and production parameters remain provisional. No production deployment or funds are authorized by the current design record.

## Validation

Run checks for every component touched:

```bash
cd contracts && forge fmt --check && forge test
cd frontend && npm ci && npm run lint && npm run build
cd cli && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked
cd spikes/gnosis-anchor && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked
cd spikes/snapshot-bench && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked
./scripts/check-brand.sh
cmp docs/listing-policy.md frontend/public/listing-policy.md
```

See [`docs/DESIGN.md`](docs/DESIGN.md) for the product architecture and [`docs/BRAND.md`](docs/BRAND.md) for naming conventions.