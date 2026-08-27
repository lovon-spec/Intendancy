# snapshot-bench

Non-production spike for [Implementation Brief 0002](../../docs/implementation-briefs/0002-snapshot-benchmarks.md)
(RFC 0001 **Gate 2**): measure Classic-GTCR snapshot generation and verification at
1,000 / 10,000 items, plus the CAR/UnixFS install path. Results, environment pin,
and freeze recommendations: [`docs/spikes/snapshot-bench-results.md`](../../docs/spikes/snapshot-bench-results.md).

The anchor stateRoot is a TRUSTED INPUT here — anchor derivation is Gate 1's scope
(`spikes/gnosis-anchor`). This crate deliberately has no consensus/BLS dependencies.
Verification is nevertheless PROFILE-BOUND: `verify` requires a pinned
`VerifierProfile` (version, chain, registry, runtime codehash, anchor block
number/HASH/state root — in this bench, the seed manifest) and checks the proven
account codehash against the pin, so proofs of the wrong account under an
authentic root are rejected. Snapshot files are read through a BOUNDED file
reader: the applicable byte cap is enforced while streaming (at most cap + 1
bytes materialized), before any parse.

## Offline tests (default; no network)

```bash
cargo test --locked      # 69 tests: schema units + descriptor vectors + snapshot
                         # mutations/profile bindings + resource limits (incl. the
                         # bounded file reader) + CAR tamper
```

Candidate fixtures (in-tree, repo-untracked until an authorized commit lands):
`fixtures/manifest-25.json` + `fixtures/snapshot-25.json`
(25 items covering ALL FOUR Classic statuses, including an executed removal whose
zero status slot exercises the zero-as-absence proof branch),
`fixtures/eoa-snapshot-25.json` (the profile-binding attack artifact: an honest
empty-catalog proof for a funded EOA), and `fixtures/descriptor-vectors.json`
(independent RLP vectors from the frontend's viem encoder — see below).

## Reproducing the benchmarks

Terminal 1 — anvil fork of Gnosis, PINNED block (history pruning keeps 10k-block
seeding within memory; without it anvil dies on memory-pressured hosts):

```bash
anvil --fork-url https://rpc.gnosischain.com --fork-block-number 47881774 \
      --port 8547 --prune-history --transaction-block-keeper 16
```

Terminal 2:

```bash
cargo build --release --locked
B=./target/release/snapshot-bench
/usr/bin/time -l $B seed     --rpc-url http://127.0.0.1:8547 --items 10000 --out /tmp/m.json
/usr/bin/time -l $B generate --manifest /tmp/m.json --out /tmp/s.json
gzip -kf /tmp/s.json
/usr/bin/time -l $B verify   --manifest /tmp/m.json --snapshot /tmp/s.json      # raw JSON
$B verify   --manifest /tmp/m.json --snapshot /tmp/s.json.gz                    # gzip transport form
$B car-bench --files 6 --file-bytes 8192   --work /tmp/car-small
$B car-bench --files 4 --file-bytes 262144 --work /tmp/car-large
```

`seed` deploys a fresh registry through the REAL `GTCRFactory`
(`0x794Cee5a…FE039`; deployed codehash matches the Gate 1 canary generation),
submits synthetic entries in the V1 six-column descriptor encoding, and drives the
first three items through the real state machine (execute / remove / execute) so
every Classic status has proof coverage. Anvil's fork mode carries no state root in
block headers; the manifest's trusted root is the root NODE of a probe proof
(`keccak256(accountProof[0])`) — documented in the results. Note anvil's fork-mode
trie represents zeroed slots as explicit `RLP(0x80)` leaves rather than deleting
them; the verifier accepts both zero forms (see results §1).

`car-bench --expected-cid <cid>` overrides the expected Tree CID input — the
consumer path verifies against the EXTERNAL expected CID (in production: the
verified descriptor's), never the CAR's own root claim, so a mismatched value
demonstrates the refusal.

## Regenerating the candidate fixtures

With the pinned anvil running (fresh fork):

```bash
cargo run --release --locked -- seed      --rpc-url http://127.0.0.1:8547 --items 25 --out fixtures/manifest-25.json
cargo run --release --locked -- generate  --manifest fixtures/manifest-25.json --out fixtures/snapshot-25.json
cargo run --release --locked -- eoa-probe --manifest fixtures/manifest-25.json --out fixtures/eoa-snapshot-25.json
node tools/gen-descriptor-vectors.mjs   # needs frontend/node_modules (viem)
```

`tools/gen-descriptor-vectors.mjs` loads the repo's real
`frontend/src/lib/encoder.ts` (node's native TypeScript type stripping; a resolve
hook supplies the bundler-style missing `.ts` extensions) and emits cross-
implementation descriptor vectors with provenance (viem version, node version,
source per vector). Rust must byte-match them in both directions — this is the
descriptor-freeze evidence.

## Notes

- The CAR/UnixFS implementation is a self-contained NON-PRODUCTION prototype
  (sha2-256 CIDv1; raw + dag-pb codecs; plain Directory nodes + single-block raw
  leaves; shared blocks supported). Chunked multi-block File nodes and HAMT
  directories are NOT supported, and no kubo interop has been demonstrated — the
  install phase's production verdict is a precise NO-GO gated on kubo vectors
  (results §4). The consumer path is still strict: external expected CID, full
  preflight before any write, staged install published by one atomic rename,
  nothing pre-existing at the destination (symlinks included), materialized-byte
  policy cap.
- Limit VALUES are working values; they freeze with the binary transport framing
  (results §3/§6).
