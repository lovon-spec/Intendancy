# Gnosis Finalized-State Anchor Spike — Results

**Brief**: [`docs/implementation-briefs/0001-gnosis-light-client-spike.md`](../implementation-briefs/0001-gnosis-light-client-spike.md)
**Crate**: `spikes/gnosis-anchor/`
**Date**: 2026-08-23
**Conclusion**: **GO** — see §8. The strict anchor chain works end to end against live Gnosis with unmodified upstream verification code; the only Gnosis-specific artifacts are a data-level spec/preset injection and one 4-line local helper replacing an upstream function that hardcodes mainnet slot timing.

## 1. Dependency and revision choices

| Dependency | Version/revision | Why |
|---|---|---|
| `helios-consensus-core` | git `a16z/helios` @ `43a8c9f3cdda41a6f383c4db41d9a83f102638b1` | The consensus light-client verification core. Inspection showed every verification entry point (`verify_bootstrap`, `verify_update`, `verify_finality_update`, `apply_*`) is **generic over a public `ConsensusSpec` trait**, and the fork schedule (`Forks`) plus genesis root are plain data parameters — so Gnosis support needs **no fork of upstream** (§4). Pinned by exact revision; `Cargo.lock` committed. |
| `alloy` (requested 1.0.37, **resolved 1.8.3**) / `alloy-trie` (requested & **resolved 0.9.5**, `ethereum` feature; helios itself requests 0.9.1 which also resolves 0.9.5) | Same family/feature set the pinned helios revision uses | EIP-1186 verification mirrors the reviewed pattern of helios `core/src/execution/proof.rs` (`alloy_trie::proof::verify_proof`, keccak-pathed keys, RLP `TrieAccount` leaf, exclusion proofs for empty values) without pulling helios-core's full dependency tree. No bespoke MPT or BLS code anywhere in the spike: BLS12-381 verification comes transitively from `helios-consensus-core` (`bls12_381` 0.8). |
| `reqwest` (blocking, rustls), `serde`/`serde_json`, `clap`, `eyre`, `thiserror`, `tree_hash`, `typenum` | exact versions in `Cargo.lock` | Transport, bounded JSON handling, CLI, error handling. |

All reproduction commands run `--locked` against the committed `Cargo.lock`; the resolved-version column above is taken from that lockfile, not from requested ranges.

**Error-type design (brief: "explicit error types")**: every failure that *decides trust* — invalid live configuration, genesis mismatch, unsupported fork version, rejected bootstrap/update/finality, stale checkpoint, insufficient participation, no signed advancement, account/storage proof failure, codehash mismatch, undecodable status, invalid high-water state, high-water violation — is a variant of the bounded `StageError` enum (`src/errors.rs`). `eyre` remains in the transport/CLI plumbing for context propagation only; it never makes a trust decision.

**Library-boundary invariants (re-review blockers)**: `pipeline::validate()` runs before ANY network access and enforces, in the library (not merely the CLI): live mode requires a persistent state file; live mode refuses fixed replay time; the state-file leaf must not be a symlink; and the state file must not be equal to or inside the capture directory. Validation first creates the configured capture directory and state-file parent, then canonicalizes both against the **real filesystem** before comparison. This ordering resolves dot-dot hops, existing and formerly dangling parent symlinks, on-disk case, and filesystem normalization before response capture can activate a future alias; state-leaf symlinks and metadata errors fail typed even when capture is disabled. `HighWater::load` independently rejects leaf symlinks and metadata errors rather than interpreting them as an empty first run. `--capture-dir` is live-only (clap conflict with `--offline-dir`), so offline replay cannot write over its own inputs or state. The high-water state file is **versioned (`version: 1`) and strictly parsed** (`deny_unknown_fields`, required fields): foreign JSON such as a capture `meta.json`, an empty object, or a future version is a typed `InvalidHighWaterState` error, never silently treated as an empty database. Every raw response, `meta.json`, and high-water state update uses one shared primitive: a uniquely named temporary file opened with `create_new`, followed by same-directory atomic rename. Thus neither destination links nor pre-existing/predicted temporary-path symlinks and hardlinks are followed; high-water state is the **final writer** for additional defense in depth. Post-validation local path replacement remains outside this non-production spike's trusted-local-filesystem boundary.

**Time semantics (re-review finding 3)**: live verification reads the trusted **local clock immediately before every** `verify_update`/`verify_finality_update` call — on 5-second Gnosis slots, a `now` frozen at pipeline start can legitimately fall behind a fresh signature slot across HTTP round trips (observed live in review). Offline replay uses a fixed captured time for determinism (`TimeSource::FixedForReplay`); provider-reported time is never consulted and no tolerance is applied.

## 2. Gnosis parameters and their sources

Pinned in `src/gnosis.rs`, derived from:

- `gnosischain/configs` @ `e542d132340e68fd7922149b145a0d361e1c87d4` — `mainnet/config.yaml`: fork epochs/versions (genesis `0x00000064` … Electra @ epoch 1,337,856, **Fulu @ epoch 1,714,688**, GLOAS unscheduled), `SECONDS_PER_SLOT: 5`, preset base `gnosis`.
- **SSZ container bounds**: `gnosischain/specs` @ `045d46d6db96a39b4d91485f9783474c13546ac9`, preset files `consensus/preset/gnosis/{phase0,altair,bellatrix,capella,deneb,electra,fulu}.yaml` (and `consensus/config/gnosis.yaml`) — source of the `GnosisConsensusSpec` bounds including `SLOTS_PER_EPOCH 16`, `EPOCHS_PER_SYNC_COMMITTEE_PERIOD 512`, `SYNC_COMMITTEE_SIZE 512`, and `MAX_WITHDRAWALS_PER_PAYLOAD 8` (the only container-bound delta vs mainnet).
- Live cross-check (never authoritative): `/eth/v1/beacon/genesis` — genesis time `1638993340`, genesis validators root `0xf5dcb5564e829aab27264b9becd5dfaa017085611224cb3036f573368dbb9d47` — is captured as a fixture and enforced by the runtime **genesis cross-check stage**, which hard-fails if an endpoint disagrees with the pinned constants (wrong-network defense). The endpoint's full `/eth/v1/config/spec` dump is additionally recorded at `fixtures/config-spec.json` as a provenance artifact; the verifier itself never fetches or consumes it.

A sync-committee period on Gnosis is 16 × 512 = 8,192 slots × 5 s ≈ **11.4 hours** (vs ~27 h mainnet).

## 3. Fixture provenance (canary)

| Item | Value | Verified how |
|---|---|---|
| Registry | `0x54A92C21c6553a8085066311F2C8D9Db1B5e6610` | `GTCRFactory(0x794Cee5a6e1501b633eC13b8c1e327d9860FE039).instances(3)` returns it (checked live via `cast call`) — an official-factory Classic GeneralizedTCR deployment |
| Factory deployment event | `NewGTCR(address)` at factory `0x794Cee5a…FE039`, **block 17,179,159** (hash `0x759e26e97948d0222bf623b6d06f38a0a0af6177529b6345913fbdb564a945b5`), tx `0x7e4aed6d917dd5093271b5655eac7d2698b0774ebbe8e0b575c1c69a9d69606d`, **log index 30**, topic carrying `0x54a92c21…6610` | Independently re-verified from the transaction receipt (`cast receipt`; status `0x1`; the same tx also deploys instance 2 at log index 26) |
| Runtime codehash | `0x5a6cf79325018f60d2aa63ca57c5396ae760b2ae57d4572c631778b3e9085d7d` | `keccak256(eth_getCode(registry))` recomputed live; equals the brief's seed and is the locally pinned profile value |
| Storage layout | `itemList` at slot 13, `items` mapping at slot 14; `items[id].status` at base+1 (whole word must be 0..=3) | State-variable order read from `kleros/tcr` `contracts/GeneralizedTCR.sol` @ `72e547ea135d839dc5db34e79e9f94f05c6a92bb`; binding to the *deployed* bytecode is through the pinned runtime codehash gate plus empirical verification — the proven slot-13 value (27) matches the registry's `itemCount()`, and `itemList[0]`'s proven value is the first itemID |
| Proven slots | slot `0x0d` (`itemList.length`) and `keccak256(uint256(13)) + 0` (`itemList[0]`) | Slot keys derived **locally** in `src/profile.rs`; provider-supplied keys are never used |

## 4. The Gnosis-support delta against upstream (the brief's central question)

**Current Helios does select an Ethereum spec at compile time** (`MainnetConsensusSpec`), confirming the brief's suspicion that a TOML file cannot change SSZ bounds. However, the compile-time spec is an *instantiation*, not a constraint: `helios-consensus-core` exposes `ConsensusSpec` as a public trait and every verification function is generic over it. The complete Gnosis delta is therefore, in this spike, **data**:

1. `GnosisConsensusSpec` — a trait impl carrying the gnosis preset (16-slot epochs, 512-epoch periods, `MaxWithdrawals = 8`, …): ~40 lines.
2. `forks()` — the Gnosis fork schedule as a `Forks` value: ~15 lines.
3. Genesis constants (time, validators root).
4. One **local replacement helper**: upstream `expected_current_slot` hardcodes `since_genesis / 12` (mainnet seconds-per-slot) and MUST NOT be used for Gnosis; the spike computes `(now − genesis_time) / 5` locally (`src/gnosis.rs::expected_current_slot`).

**Upstream patch candidates** (small, upstreamable; neither blocks the spike):
- Parametrize slot duration — e.g. add `SecondsPerSlot` to `ConsensusSpec` and make `expected_current_slot` generic (the only mainnet-hardcoded timing found on the verification path).
- Optionally ship a `GnosisConsensusSpec` + Gnosis network config upstream so embedded Helios clients (not just the core crate) can target Gnosis.

Additional hardening added above the library: `verify_generic_update` accepts any nonzero sync participation; the spike separately **requires ≥ 2/3 supermajority** on the finality update before adopting an anchor.

Fork-schema handling: the live endpoint serves `version: "fulu"` payloads; decode is allow-listed (`capella`–`fulu`) and **fails closed on unknown versions** (e.g. a future `gloas`) before any payload parsing. Fulu-era light-client containers reuse the Electra layout (7-node finality branch, post-Electra committee-branch depths), which upstream handles via its fork-parameterized proof depths.

## 5. Reproduction

Live (writes fixtures + high-water state; checkpoint MUST be an explicit finalized beacon block root ≥ 1 period old):

```bash
cd spikes/gnosis-anchor
cargo run --release --locked -- \
  --checkpoint 0x28120630451d1d5a842fd5e9b19c8b30bb9ea4928a8a06e8a1f50e0cbce0ea5f \
  --consensus-rpc https://rpc-gbc.gnosischain.com \
  --execution-rpc https://gnosis-rpc.publicnode.com \
  --capture-dir fixtures --state-file /tmp/gnosis-anchor-hw.json --json
```

(`--state-file` is mandatory in live mode, must lie outside `--capture-dir`, and `--now-unix` is refused there. The checkpoint above is the recorded fixture checkpoint — beacon slot 29,691,984; substitute any newer finalized root at least one sync-committee period old.)

The informational `fixtures/config-spec.json` provenance snapshot is regenerated with:

```bash
curl -sS https://rpc-gbc.gnosischain.com/eth/v1/config/spec -o fixtures/config-spec.json
```

Offline (default; deterministic; no network):

```bash
cargo test --locked   # unit + offline end-to-end + adversarial suites
```

Endpoint notes: `rpc.gnosischain.com` (execution) does not serve `eth_getProof` at the depths needed (confirming the brief's warning); `gnosis-rpc.publicnode.com` and `gnosis.drpc.org` both do (independently verified). The consensus and execution endpoints are separately configurable for exactly this reason. Checkpoint used for the recorded run: slot 29,691,984 — **~1.5 sync-committee periods before the finality target**, so the run demonstrates verified sync-committee advancement, not just bootstrap acceptance.

## 6. Live transcript summary

Recorded capture run (2026-08-23, all stages ok; raw responses checked in under `fixtures/`):

| Stage | Result |
|---|---|
| genesis-crosscheck | endpoint genesis matches pinned Gnosis values |
| consensus-anchor | finalized beacon slot **29,704,320** via **3 verified sync-committee updates**; participation **504/512** |
| proof-fetch | `eth_getProof` at execution block **47,878,436** (2 slots) |
| account-proof | verified against stateRoot; storageRoot `0xfef98601…1057` |
| codehash-pin | matches pinned `0x5a6cf793…5d7d` |
| storage-proof | slot `0x0d` → **`0x1b` (itemCount 27** — the brief's expected canary value**)**; `itemList[0]` → itemID `0x8fd14b8b…fc28` |
| high-water | Initialized (a second run minutes later: **Advanced**, demonstrating the monotonic path live) |

Post-remediation fresh live run (2026-08-23, after the re-review fixes; all 8 stages ok including the new `config` invariants stage): finalized beacon slot **29,705,232**, execution block **47,879,332** (hash `0xb00aefb2…4e0f`, stateRoot `0x22744305…3473`), participation 498/512, 3 verified updates, 5.3 s wall / 32 MB RSS, state file written in the strict `version: 1` schema. The reviewer's independent run additionally verified slot 29,704,576 / block 47,878,691. Final Codex-owned verification, using initially absent and separate capture/state directories, passed all 8 stages at beacon slot **29,706,112** / execution block **47,880,185** (hash `0xdbaef73e2eede64d4be70ab6a397e9626bbec50c33c7bef50d1970624e5873d3`, stateRoot `0xcee1c15ccf45404068f37ce53ec788256d5e3a494a4ecc965a112e497d65f973`); both capture metadata and strict version-1 high-water state were present afterward. After the final shared-writer hardening, a fresh run through atomic capture **and** high-water persistence again passed all 8 stages at slot **29,706,256** / block **47,880,323** (hash `0x7b959b20436a2605cf8f8c7fd51e8a945cf83aca239cda3b8c6b5c3f9f6ecc57`, stateRoot `0xfc5be416abe6f28231ddcd077a46b14362739a7e6a73f61e0cbffde80b8e06fa`); all six capture files and strict high-water state were present, with no temporary files left behind.

Anchor identifiers: checkpoint `0x28120630451d1d5a842fd5e9b19c8b30bb9ea4928a8a06e8a1f50e0cbce0ea5f` (beacon slot 29,691,984; age 61,890 s ≈ 17.2 h ≈ **1.5 sync-committee periods before the target** — genuine signed advancement, not bootstrap acceptance); finalized execution block hash `0xbc68ba18a1cd3037f09b9cf6067c106f4e78c599b21b6715dde29984a1a0c8d2`, stateRoot `0xe61693012c82d3915b0bc970ce173fcd09587e9b99a71d1e8e4730958cd6cd66`, storageRoot `0xfef986013933b8b6068507898f791b155551e3064797cbbdae15d4dac8a61057`, timestamp 1787514940.

**Measurements** (release build, macOS arm64):

- Wall time per full live verification: **3.2–6.2 s**, network-dominated (user+sys CPU ≈ **0.13 s** — BLS + MPT verification cost is milliseconds).
- Peak RSS: **31–38 MB**. Binary size: **6.1 MB** (unstripped).
- Bytes downloaded (spec-compliant server case): bootstrap ~54 KB + the 3 needed updates ~172 KB + finality update ~5 KB + genesis ~170 B + proof ~11 KB ≈ **~243 KB** per cold anchor. (The ~290 KB updates *fixture* is the trimmed 5-entry form — 3 needed + 2 retained backfill extras; the live `rpc-gbc` endpoint actually shipped ~7 MB for that request due to the backfill quirk below.)

**Endpoint quirk discovered (first live run failed on it, by design):** `rpc-gbc.gnosischain.com` answers `light_client/updates?start_period=P&count=N` with the N requested updates **followed by its entire historical backfill** (122 entries observed, reaching back ~75 days). The pipeline initially fail-closed on the out-of-window extras; the fix truncates to the requested window before decode (extras are discarded unexamined — never trusted, never fatal). The committed updates fixture is the raw response trimmed to 5 entries (3 requested + 2 backfill extras) so offline replay still exercises the truncation path; the regeneration command in §5 re-captures the full raw form.

## 7. Offline and adversarial test results

`cargo test --locked --offline` (deterministic — no network): **43/43 passing** in <2 s after compilation. Lint gate: `cargo clippy --all-targets --locked --offline -- -D warnings` (exit 0); format gate: `cargo fmt --check`.

- `tests/offline.rs` (2): full pipeline from checked-in fixtures reproduces the recorded anchor exactly (execution block/hash/stateRoot, itemCount 0x1b, ≥2/3 participation, signed advancement past the bootstrap slot), and two replays produce byte-identical reports.
- `tests/adversarial.rs` (16), each asserting fail-closed rejection with a matching diagnostic:
  wrong checkpoint/bootstrap root · corrupted sync-committee signature · zeroed participation · corrupted finality branch · corrupted execution branch · substituted execution stateRoot · **tampered authenticated execution block number** · **tampered authenticated execution block hash** · unknown fork version (`gloas`) refused before decode · Ethereum-mainnet constants (spec + fork versions + genesis root) fail to verify Gnosis data · malformed account proof · wrong locally pinned codehash · corrupted storage value · stale checkpoint · anchor rollback · conflicting block hash at the same height.
- Unit tests (13): status decode (0..=3 named; any other value — garbage upper bytes, `U256::MAX` — is a typed `UndecodableStatus` **error**); the ≥2/3 supermajority threshold helper (**341/512 fails typed `InsufficientParticipation`, 342 passes** — covering the guard the zeroed-bits adversarial case cannot reach, since upstream signature verification rejects first); `TimeSource` semantics; five strict high-water state regressions (CaptureMeta-shaped JSON rejected, `{}` rejected, wrong version rejected, state-leaf symlink rejected, versioned round-trip works); a state-last persistence regression using a static hardlink alias; an atomic capture regression proving early raw-response writes neither follow fixture symlinks nor mutate hardlinked rollback state, including the failure-before-persist case; a shared-writer regression proving predicted unique-temp symlink and hardlink collisions are skipped without touching their victims; and an integration regression for the former predictable `highwater.json.tmp` symlink exploit.
- `tests/adversarial.rs` totals **27 = 16 brief attack cases + 11 configuration/aliasing invariants**: live without a state file; live with fixed time; dangling state-leaf symlink with capture disabled; state file equal to the capture meta path; dot-dot hop; existing parent symlink; an inside-capture state-leaf symlink pointing outside; case alias on case-insensitive storage; formerly dangling relative and absolute parent symlinks activated by capture-directory creation; and NFC/NFD normalization aliasing on filesystems that implement it. Each applicable alias is rejected by `pipeline::validate()` **before any network access** (the test endpoints are unreachable, proving pre-transport rejection).
- `tests/cli.rs` (1): `--offline-dir` + `--capture-dir` is a clap conflict — offline replay can never write over its own inputs (the round-3 offline-state-destruction reproduction).
- The high-water report line carries an explicit `[persisted]` / `[EPHEMERAL …]` grade; an explicit `--now-unix` is always honored as fixed time so using it live produces the typed `LiveFixedTimeForbidden` error rather than a silent ignore.
- Period-boundary decoding is exercised by the offline fixtures themselves: the three verified updates span sync-committee periods 3624 → 3626 (two boundary crossings) under the active `fulu` fork schema.
- `cargo clippy --all-targets --locked --offline -- -D warnings`: clean. `cargo fmt --check`: clean. The CLI offline mode (`--offline-dir fixtures`) reproduces the same all-stages-ok report.

Not covered by tests (documented, not hidden): live fork-activation transitions (no activation is scheduled to record); Beacon API implementations other than the official Gnosis endpoint; bootstrap retention limits for very old checkpoints; resource-exhaustion behavior beyond the 16 MiB response bound.

## 8. Conclusion and remaining trust assumptions

**GO.** The strict chain — explicit checkpoint → verified sync-committee advancement → finalized execution `stateRoot` → EIP-1186 account/storage proofs → pinned-codehash Classic GTCR reads — works against live Gnosis using unmodified, pinned upstream verification code, with the Gnosis delta expressible as data plus a 4-line timing helper. No `NO-GO` findings; one upstream improvement identified (slot-duration parametrization) that would let future work delete the local helper.

Remaining trust assumptions, stated explicitly:

1. **Weak subjectivity**: the operator-supplied `--checkpoint` is socially canonical. No mechanism here can validate it; distribution of checkpoints (signed releases, operator quorum) is out of scope per the brief.
2. **Gnosis consensus honesty**: ≥ 2/3 of the sync committee is honest (inherent to the light-client protocol); the spike's supermajority floor enforces the quorum on the finality update it adopts.
3. **Pinned constants**: the fork schedule, genesis values, preset bounds, registry address, runtime codehash, and storage layout in the local profile are correct (provenance above); layout constants are meaningless for other bytecode and are gated on the codehash match.
4. **Library correctness**: `helios-consensus-core` (BLS, SSZ, tree-hash, proof-depth logic) and `alloy-trie` at their pinned revisions verify what they claim. No cryptographic code was written in this spike.
5. **Not covered by design** (spike non-goals): checkpoint distribution, header-quorum/RPC fallback modes, snapshot formats and benchmarks, CAR/UnixFS content verification, resource-exhaustion hardening beyond the 16 MiB response bound, and any production packaging.

Known limitations / not tested: live behavior across a fork *activation* boundary (Fulu activated 2026-04-14; the next fork, GLOAS, is unscheduled — the fail-closed version allow-list is the guard); Beacon API providers other than `rpc-gbc.gnosischain.com`; bootstrap availability for checkpoints much older than the endpoint's retention.

Residual production notes (documented, deliberately not fixed in this spike):

- High-water persistence is atomic (temp file + rename) but **not concurrency-safe across processes** — two concurrent runs sharing a state file can interleave check-then-write.
- Path validation prepares and canonicalizes configured output directories before network access, but it is not a production secure-open primitive; a privileged local actor can still race path replacement after validation. Static capture-child and temporary-path symlink/hardlink aliases are harmless because both capture and high-water persistence use create-new temporary files plus atomic rename rather than destination writes; high-water state is also the final writer.
- Operator-supplied RPC URLs are echoed verbatim into reports and transport errors; URLs embedding API keys would leak into logs. Production tooling should redact.
- The 16 MiB response bound applies to the raw body *before* backfill truncation; a server whose appended backfill outgrows the cap makes the request fail closed (an availability problem, never a trust problem). Production clients should page or renegotiate rather than inherit this bound.
- Correct local wall time is a live-mode trust assumption (checkpoint-age and expected-slot checks depend on it); `--now-unix` exists only for offline replay and is refused live.
