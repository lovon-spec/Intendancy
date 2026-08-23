# gnosis-anchor-spike

Non-production spike for [Implementation Brief 0001](../../docs/implementation-briefs/0001-gnosis-light-client-spike.md):
prove that a small consumer can go from an **explicit Gnosis weak-subjectivity checkpoint**
to a **consensus-verified finalized execution `stateRoot`**, and verify **EIP-1186
account/storage proofs** for a Classic GeneralizedTCR against it — with every input
beyond the checkpoint treated as untrusted.

Results, provenance, and the GO/NO-GO conclusion live in
[`docs/spikes/gnosis-anchor-results.md`](../../docs/spikes/gnosis-anchor-results.md).

## Verification chain

```
explicit --checkpoint (beacon block root)
  -> GET light_client/bootstrap        (untrusted)  -> verify_bootstrap
  -> GET light_client/updates          (untrusted)  -> verify_update × N   (signed advancement)
  -> GET light_client/finality_update  (untrusted)  -> verify_finality_update
  -> finalized EXECUTION header {number, hash, stateRoot, timestamp}   (authenticated)
  -> eth_getProof at that exact block  (untrusted)  -> MPT-verify account vs stateRoot
  -> require locally pinned runtime codehash
  -> MPT-verify locally derived Classic GTCR slots vs storageRoot
  -> per-registry finalized-anchor high-water check
```

Gnosis support is injected as data (`src/gnosis.rs`: a `ConsensusSpec` impl with the
gnosis preset + the pinned fork schedule); upstream helios-consensus-core is consumed
unmodified at a pinned revision.

## Offline (default, deterministic)

```bash
cargo test --locked            # offline pipeline + adversarial suite; no network
```

Replay the checked-in fixtures manually:

```bash
cargo run --locked -- \
  --checkpoint 0x28120630451d1d5a842fd5e9b19c8b30bb9ea4928a8a06e8a1f50e0cbce0ea5f \
  --offline-dir fixtures \
  --json
```

(The checkpoint is the recorded fixture checkpoint from `fixtures/meta.json`.)

## Live

```bash
cargo run --locked -- \
  --checkpoint 0x28120630451d1d5a842fd5e9b19c8b30bb9ea4928a8a06e8a1f50e0cbce0ea5f \
  --consensus-rpc https://rpc-gbc.gnosischain.com \
  --execution-rpc https://gnosis-rpc.publicnode.com \
  --max-checkpoint-age 172800 \
  --state-file /tmp/gnosis-anchor-highwater.json \
  --json
```

Substitute any newer finalized beacon block root at least one sync-committee period
(~11.4 h) old for the checkpoint; `https://gnosis.drpc.org` is a verified alternative
execution RPC (it must serve `eth_getProof` at finalized depth).

The state file must NOT live inside `--capture-dir`, and its leaf must be a regular path,
not a symlink. Before network access, the pipeline prepares and canonicalizes the output
locations so dot-dot, case/normalization, and parent-symlink aliases are rejected; state
leaf symlinks also fail closed when capture is disabled. Every captured response and
`meta.json` is installed via a create-new temporary file plus atomic rename, so a static
fixture symlink or hardlink is replaced rather than followed. The high-water file is
persisted last through the same create-new/atomic-rename primitive as additional defense
in depth; predictable temp-path links are never opened.

Add `--capture-dir fixtures` to (re)record the fixture set; `fixtures/meta.json` then
pins the capture time so offline replay is deterministic. Note: `rpc.gnosischain.com`
does not retain `eth_getProof` at finalized depth; use an execution provider that does
(the consensus and execution endpoints are independently configurable for this reason).

There is no automated live test target; the explicit live CLI invocation above IS the
live test for this spike, and its recorded transcript lives in the results document.

## Security posture (spike)

- `--checkpoint` is mandatory; there is no checkpoint auto-download or default.
- `--state-file` is mandatory in live mode: rollback protection must be persistent to
  mean anything. `--now-unix` is refused in live mode (offline-replay determinism only);
  correct local wall time is a documented trust assumption of live verification.
- Consensus/execution endpoints are untrusted; agreement with an RPC `finalized` tag is
  never treated as verification.
- Target address, expected runtime codehash, storage layout, and slot keys come from the
  local fixture profile (`src/profile.rs`), never from provider output.
- Unknown light-client fork versions fail closed before payload decode.
- Responses are size-bounded; results are staged, never a single boolean.
